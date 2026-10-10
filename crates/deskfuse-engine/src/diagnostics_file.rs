//! Bounded, non-blocking diagnostic persistence. Only metadata records enter this
//! queue; payloads and input events are never serialized here.
use crate::diagnostics::DiagnosticRecord;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const HOUR: u64 = 3_600;
const RETENTION: u64 = 7 * 24 * HOUR;
static WRITER: OnceLock<Option<SyncSender<DiagnosticRecord>>> = OnceLock::new();
static DROPPED: AtomicU64 = AtomicU64::new(0);

pub(crate) fn initialize() {
    WRITER.get_or_init(start);
}

pub(crate) fn record(record: &DiagnosticRecord) {
    if let Some(sender) = WRITER.get_or_init(start)
        && sender.try_send(record.clone()).is_err()
    {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}

fn directory() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|home| home.join("Library/Logs/DeskFuse"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        dirs::data_local_dir().map(|home| home.join("DeskFuse/logs"))
    }
}

fn start() -> Option<SyncSender<DiagnosticRecord>> {
    let directory = directory()?;
    let (sender, receiver) = mpsc::sync_channel::<DiagnosticRecord>(256);
    std::thread::Builder::new()
        .name("deskfuse-log-writer".into())
        .spawn(move || {
            let mut output: Option<(u64, File)> = None;
            if let Err(error) =
                prepare_directory(&directory).and_then(|()| prune(&directory, seconds_now()))
            {
                eprintln!(
                    "DeskFuse: initial log maintenance failed ({:?})",
                    error.kind()
                );
            }
            let mut last_cleanup = Some(seconds_now());
            loop {
                // Wake even when logging is disabled or the app is idle.
                let received = receiver.recv_timeout(Duration::from_secs(60));
                let now = seconds_now();
                if last_cleanup.is_none_or(|last| now < last || now.saturating_sub(last) >= 60) {
                    if let Err(error) =
                        prepare_directory(&directory).and_then(|()| prune(&directory, now))
                    {
                        eprintln!("DeskFuse: log maintenance failed ({:?})", error.kind());
                    }
                    last_cleanup = Some(now);
                }
                let record = match received {
                    Ok(record) => record,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                };
                let hour = record
                    .timestamp
                    .split('.')
                    .next()
                    .and_then(|seconds| seconds.parse::<u64>().ok())
                    .unwrap_or(now)
                    / HOUR;
                // A delayed queue entry must never recreate an expired slice.
                if hour * HOUR <= now.saturating_sub(RETENTION) {
                    continue;
                }
                if output.as_ref().is_none_or(|(current, _)| *current != hour) {
                    // UTC epoch-hour names do not collide when local DST changes.
                    let path = directory.join(format!("diagnostics-{hour}.jsonl"));
                    output = match open(&path) {
                        Ok(file) => Some((hour, file)),
                        Err(error) => {
                            eprintln!("DeskFuse: log slice unavailable ({:?})", error.kind());
                            None
                        }
                    };
                }
                let Some((_, file)) = output.as_mut() else {
                    continue;
                };
                let dropped = DROPPED.swap(0, Ordering::Relaxed);
                let result = (|| -> std::io::Result<()> {
                    if dropped > 0 {
                        let mut overflow = serde_json::to_vec(&serde_json::json!({
                            "timestamp_seconds": now, "subsystem": "diagnostics",
                            "event": "bounded_queue_overflow", "dropped_records": dropped
                        }))?;
                        overflow.push(b'\n');
                        file.write_all(&overflow)?;
                    }
                    // Parent and input helper append to the same hourly file.
                    // Append each complete JSONL line in one write rather than
                    // interleaving the serializer's fragments across processes.
                    let mut line = serde_json::to_vec(&record)?;
                    line.push(b'\n');
                    file.write_all(&line)
                })();
                if let Err(error) = result {
                    eprintln!("DeskFuse: log write failed ({:?})", error.kind());
                    output = None;
                }
            }
        })
        .ok()?;
    Some(sender)
}

fn seconds_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn prepare_directory(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn open(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn prune(directory: &Path, now: u64) -> std::io::Result<()> {
    let cutoff = now.saturating_sub(RETENTION);
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let expired = if let Some(hour) = name
            .strip_prefix("diagnostics-")
            .and_then(|name| name.strip_suffix(".jsonl"))
            .and_then(|hour| hour.parse::<u64>().ok())
        {
            // Remove the entire slice when its oldest entry reaches seven days.
            hour.saturating_mul(HOUR) <= cutoff
        } else if name == "panic-first.log" {
            entry
                .metadata()?
                .modified()?
                .duration_since(UNIX_EPOCH)
                .is_ok_and(|time| time.as_secs() <= cutoff)
        } else {
            false
        };
        if expired {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}
