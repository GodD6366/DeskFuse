//! Coalesced, bounded maintenance. Never run this work on the pasteboard worker.
use super::*;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Condvar, Weak};
use std::time::{Duration, Instant};

const MAX_PENDING_USES: usize = 1024;

#[derive(Default)]
struct Pending {
    used: BTreeMap<u64, u64>,
    wake: bool,
}

#[derive(Default)]
pub(super) struct Queue {
    pending: Mutex<Pending>,
    ready: Condvar,
    running: AtomicBool,
    cleanup: AtomicBool,
}

impl Queue {
    pub(super) fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }
    pub(super) fn used(&self, id: u64) {
        if let Ok(mut pending) = self.pending.lock() {
            if pending.used.len() >= MAX_PENDING_USES
                && !pending.used.contains_key(&id)
                && let Some(oldest) = pending
                    .used
                    .iter()
                    .min_by_key(|(_, time)| *time)
                    .map(|(id, _)| *id)
            {
                pending.used.remove(&oldest);
            }
            pending.used.insert(id, now_ms());
            pending.wake = true;
            self.ready.notify_one();
        }
    }
    pub(super) fn cleanup(&self) {
        self.cleanup.store(true, Ordering::Release);
        self.wake();
    }
    pub(super) fn wake(&self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.wake = true;
            self.ready.notify_one();
        }
    }
}

pub(super) fn start(library: &Arc<Library>) {
    let queue = library.work.clone();
    if queue.running.swap(true, Ordering::AcqRel) {
        return;
    }
    let library = Arc::downgrade(library);
    let failed = queue.clone();
    if std::thread::Builder::new()
        .name("deskfuse-clipboard-maintenance".into())
        .spawn(move || run(library, queue))
        .is_err()
    {
        failed.running.store(false, Ordering::Release);
    }
}

fn run(library: Weak<Library>, queue: Arc<Queue>) {
    let mut next_payload = Instant::now();
    let mut next_prune = Instant::now() + Duration::from_secs(60);
    loop {
        let uses = {
            let Ok(pending) = queue.pending.lock() else {
                return;
            };
            let Ok((mut pending, _)) =
                queue
                    .ready
                    .wait_timeout_while(pending, Duration::from_millis(25), |p| !p.wake)
            else {
                return;
            };
            pending.wake = false;
            std::mem::take(&mut pending.used)
        };
        let Some(library) = library.upgrade() else {
            return;
        };
        let mut uses = uses.into_iter().collect::<Vec<_>>();
        uses.sort_unstable_by_key(|(_, time)| *time);
        for (id, time) in uses {
            if let Err(error) = library.mark_used_at(id, time) {
                eprintln!("DeskFuse: clipboard usage update failed: {error}");
            }
        }
        // Durable publication intents survive queue pressure and process restart.
        let ids = (|| -> Result<Vec<(u64, u64)>> {
            let db = library.reader()?;
            db.prepare("SELECT id,revision FROM cl_pending_publications ORDER BY id LIMIT 4")
                .map_err(err)?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)
        })()
        .unwrap_or_default();
        for (id, revision) in &ids {
            if library.publish_index_for(*id).is_ok()
                && let Ok(db) = library.db.lock()
            {
                let _ = db.execute(
                    "DELETE FROM cl_pending_publications WHERE id=?1 AND revision=?2",
                    params![id, revision],
                );
            }
        }
        // Amortize tiny documents without running an unbounded migration loop.
        // Every transaction yields to waiting interactive writers.
        let slice = Instant::now();
        for _ in 0..64 {
            if slice.elapsed() >= Duration::from_millis(8)
                || !library.storage_step().unwrap_or(false)
            {
                break;
            }
        }
        if Instant::now() >= next_payload {
            let _ = library.migrate_payload_step();
            next_payload = Instant::now() + Duration::from_millis(100);
        }
        if queue.cleanup.swap(false, Ordering::AcqRel) {
            next_prune = Instant::now();
        }
        if Instant::now() >= next_prune {
            let more = library.prune_step().unwrap_or(false);
            let _ = library.collect_blobs_step();
            next_prune = Instant::now()
                + if more {
                    Duration::from_millis(25)
                } else {
                    Duration::from_secs(60)
                };
        }
    }
}
