//! Coalesced, bounded maintenance. Never run this work on the pasteboard worker.
use super::*;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Condvar, Weak};
use std::time::{Duration, Instant};

const MAX_PENDING_USES: usize = 1024;
const ACTIVE_WAIT: Duration = Duration::from_millis(25);
const IDLE_WAIT: Duration = Duration::from_secs(1);

fn maintenance_wait(active: bool, payload_due_in: Option<Duration>) -> Duration {
    let wait = if active { ACTIVE_WAIT } else { IDLE_WAIT };
    payload_due_in.map_or(wait, |due| wait.min(due))
}

#[derive(Default)]
struct MaintenanceStats {
    cycles: u64,
    storage_steps: u64,
    payload_steps: u64,
    failures: u64,
    yields: u64,
    last_failure: Option<&'static str>,
    last_error_kind: Option<&'static str>,
    busy_us: u64,
    max_cycle_ms: u64,
}

impl MaintenanceStats {
    fn result<T>(&mut self, stage: &'static str, result: Result<T>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(error) if error == "interrupted" => {
                // The background SQLite progress handler yields to foreground writers.
                self.yields += 1;
                None
            }
            Err(error) => {
                self.failures += 1;
                self.last_failure = Some(stage);
                self.last_error_kind = Some(maintenance_error_kind(&error));
                None
            }
        }
    }

    fn report(&mut self, diagnostics: &crate::diagnostics::Diagnostics) {
        use crate::diagnostics::DiagnosticLevel;
        crate::diagnostics::record!(
            diagnostics,
            if self.failures == 0 {
                DiagnosticLevel::Info
            } else {
                DiagnosticLevel::Warning
            },
            "clipboard_maintenance",
            "interval_summary",
            Some(format!(
                "cycles={} storage_steps={} payload_steps={} busy_ms={} max_cycle_ms={} failures={} foreground_yields={} last_failed_stage={}",
                self.cycles,
                self.storage_steps,
                self.payload_steps,
                self.busy_us / 1000,
                self.max_cycle_ms,
                self.failures,
                self.yields,
                self.last_failure.unwrap_or("none")
            )),
            self.last_error_kind.map(str::to_owned),
        );
        *self = Self::default();
    }
}

fn maintenance_error_kind(error: &str) -> &'static str {
    // Never persist arbitrary SQL, file paths, or clipboard/JSON contents.
    let error = error.to_ascii_lowercase();
    for (needle, kind) in [
        ("locked", "database_locked"),
        ("busy", "database_busy"),
        ("disk is full", "disk_full"),
        ("no space left", "disk_full"),
        ("i/o error", "io_error"),
        ("permission denied", "permission_denied"),
        ("readonly", "read_only"),
        ("read-only", "read_only"),
        ("no such table", "missing_table"),
        ("no such column", "missing_column"),
        ("constraint", "constraint_violation"),
        ("malformed", "malformed_data"),
        ("no such file", "missing_file"),
        ("invalid", "invalid_data"),
        ("syntax", "syntax_error"),
        ("eof", "unexpected_eof"),
    ] {
        if error.contains(needle) {
            return kind;
        }
    }
    "unclassified_error"
}

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
    let mut payload_pending = true;
    let mut next_prune = Instant::now() + Duration::from_secs(60);
    let mut missing_files_cursor = 0_u64;
    let mut wait = ACTIVE_WAIT;
    let mut stats = MaintenanceStats::default();
    let diagnostics = crate::diagnostics::Diagnostics::new(4);
    let mut next_report = Instant::now() + Duration::from_secs(30);
    loop {
        let uses = {
            let Ok(pending) = queue.pending.lock() else {
                return;
            };
            let Ok((mut pending, _)) = queue.ready.wait_timeout_while(pending, wait, |p| !p.wake)
            else {
                return;
            };
            pending.wake = false;
            std::mem::take(&mut pending.used)
        };
        let Some(library) = library.upgrade() else {
            return;
        };
        let started = Instant::now();
        let mut active = false;
        let mut uses = uses.into_iter().collect::<Vec<_>>();
        uses.sort_unstable_by_key(|(_, time)| *time);
        for (id, time) in uses {
            stats.result("usage_update", library.mark_used_at(id, time));
        }
        // Durable publication intents survive queue pressure and process restart.
        let ids = stats
            .result(
                "publication_query",
                (|| -> Result<Vec<(u64, u64)>> {
                    let db = library.reader()?;
                    db.prepare(
                        "SELECT id,revision FROM cl_pending_publications ORDER BY id LIMIT 4",
                    )
                    .map_err(err)?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(err)?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(err)
                })(),
            )
            .unwrap_or_default();
        for (id, revision) in &ids {
            if stats
                .result("publication", library.publish_index_for(*id))
                .is_some()
                && let Ok(db) = library.db.lock()
            {
                active = true;
                stats.result(
                    "publication_ack",
                    db.execute(
                        "DELETE FROM cl_pending_publications WHERE id=?1 AND revision=?2",
                        params![id, revision],
                    )
                    .map_err(err),
                );
            }
        }
        // Amortize tiny documents without running an unbounded migration loop.
        // Every transaction yields to waiting interactive writers.
        let slice = Instant::now();
        for _ in 0..64 {
            if slice.elapsed() >= Duration::from_millis(8) {
                break;
            }
            if !stats
                .result("storage_step", library.storage_step())
                .unwrap_or(false)
            {
                break;
            }
            active = true;
            stats.storage_steps += 1;
        }
        if Instant::now() >= next_payload {
            payload_pending = stats
                .result("payload_migration", library.migrate_payload_step())
                .unwrap_or(false);
            if payload_pending {
                active = true;
                stats.payload_steps += 1;
            }
            next_payload = Instant::now() + Duration::from_millis(100);
        }
        if queue.cleanup.swap(false, Ordering::AcqRel) {
            next_prune = Instant::now();
        }
        if Instant::now() >= next_prune {
            let more = stats.result("prune", library.prune_step()).unwrap_or(false);
            stats.result("blob_collection", library.collect_blobs_step());
            let (cursor, sweeping) = stats
                .result(
                    "missing_files",
                    library.collect_missing_files_step(missing_files_cursor),
                )
                .unwrap_or((missing_files_cursor, false));
            missing_files_cursor = cursor;
            active |= more || sweeping;
            next_prune = Instant::now()
                + if more || sweeping {
                    Duration::from_millis(25)
                } else {
                    Duration::from_secs(60)
                };
        }
        // Explicit writes still wake immediately; the timeout is a safety net
        // for durable work created without a notification, not a 40 Hz poll.
        wait = maintenance_wait(
            active,
            payload_pending.then(|| next_payload.saturating_duration_since(Instant::now())),
        );
        let elapsed = started.elapsed().as_millis() as u64;
        stats.cycles += 1;
        stats.busy_us += started.elapsed().as_micros() as u64;
        stats.max_cycle_ms = stats.max_cycle_ms.max(elapsed);
        if Instant::now() >= next_report {
            stats.report(&diagnostics);
            next_report = Instant::now() + Duration::from_secs(30);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maintenance_idle_backoff_preserves_pending_payload_deadline() {
        assert_eq!(maintenance_wait(false, None), IDLE_WAIT);
        assert_eq!(maintenance_wait(true, None), ACTIVE_WAIT);
        let due = Duration::from_millis(75);
        assert_eq!(maintenance_wait(false, Some(due)), due);
        assert_eq!(maintenance_wait(true, Some(due)), ACTIVE_WAIT);
        assert_eq!(
            maintenance_wait(false, Some(Duration::ZERO)),
            Duration::ZERO
        );
    }

    #[test]
    fn maintenance_summary_records_failures_without_payloads_and_resets() {
        let diagnostics = crate::diagnostics::Diagnostics::new(4);
        let mut stats = MaintenanceStats::default();
        assert!(
            stats
                .result::<()>("storage_step", Err("private data".into()))
                .is_none()
        );
        stats.report(&diagnostics);
        let records = diagnostics.snapshot();
        assert_eq!(records[0].error_code.as_deref(), Some("unclassified_error"));
        assert!(
            records[0]
                .result
                .as_ref()
                .unwrap()
                .contains("last_failed_stage=storage_step")
        );
        assert_eq!(
            records[0].level,
            crate::diagnostics::DiagnosticLevel::Warning
        );
        assert!(!diagnostics.to_json_lines().contains("private data"));
        assert_eq!(stats.failures, 0);
        assert!(stats.last_failure.is_none());
    }
}
