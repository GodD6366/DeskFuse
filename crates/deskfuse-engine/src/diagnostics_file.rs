//! Bounded, non-blocking diagnostic persistence. Only metadata records enter this
//! queue; payloads and input events are never serialized here.
use crate::diagnostics::DiagnosticRecord;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
            let mut last_timing_flush = Instant::now();
            let mut timing_sequence = 0;
            let mut timing_dropped = 0;
            loop {
                // Wake even when logging is disabled or the app is idle.
                let received = receiver.recv_timeout(Duration::from_secs(10));
                let now = seconds_now();
                if last_cleanup.is_none_or(|last| now < last || now.saturating_sub(last) >= 60) {
                    if let Err(error) =
                        prepare_directory(&directory).and_then(|()| prune(&directory, now))
                    {
                        eprintln!("DeskFuse: log maintenance failed ({:?})", error.kind());
                    }
                    last_cleanup = Some(now);
                }
                if last_timing_flush.elapsed() >= Duration::from_secs(10) {
                    let timing = deskfuse_platform::timing::snapshot_since(timing_sequence);
                    let next = timing.records.last().map(|record| record.sequence);
                    let missing = timing.records.first().map_or(0, |record| {
                        record.sequence.saturating_sub(timing_sequence + 1)
                    });
                    let result = (|| -> std::io::Result<()> {
                        if missing > 0 || timing.dropped_records != timing_dropped {
                            append(
                                &mut output,
                                &directory,
                                now / HOUR,
                                &serde_json::json!({
                                    "timestamp_seconds": now, "pid": std::process::id(),
                                    "subsystem": "diagnostics", "event": "input_timing_loss",
                                    "missing_since_flush": missing,
                                    "ring_dropped_total": timing.dropped_records
                                }),
                            )?;
                        }
                        for record in &timing.records {
                            // Include process identity: app and input helper share this file.
                            let mut value = serde_json::to_value(record)?;
                            value["pid"] = std::process::id().into();
                            value["version"] = env!("CARGO_PKG_VERSION").into();
                            append(&mut output, &directory, now / HOUR, &value)?;
                        }
                        Ok(())
                    })();
                    if let Err(error) = result {
                        eprintln!("DeskFuse: timing persistence failed ({:?})", error.kind());
                        output = None;
                    } else {
                        timing_sequence = next.unwrap_or(timing_sequence);
                        timing_dropped = timing.dropped_records;
                    }
                    last_timing_flush = Instant::now();
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
                if hour * HOUR <= now.saturating_sub(RETENTION) {
                    continue;
                }
                let dropped = DROPPED.swap(0, Ordering::Relaxed);
                let result = (|| -> std::io::Result<()> {
                    if dropped > 0 {
                        append(
                            &mut output,
                            &directory,
                            hour,
                            &serde_json::json!({
                                "timestamp_seconds": now, "pid": std::process::id(),
                                "subsystem": "diagnostics", "event": "bounded_queue_overflow",
                                "dropped_records": dropped
                            }),
                        )?;
                    }
                    append(&mut output, &directory, hour, &record)
                })();
                if let Err(error) = result {
                    DROPPED.fetch_add(dropped + 1, Ordering::Relaxed);
                    eprintln!("DeskFuse: log write failed ({:?})", error.kind());
                    output = None;
                }
            }
        })
        .ok()?;
    Some(sender)
}

fn append(
    output: &mut Option<(u64, File)>,
    directory: &Path,
    hour: u64,
    record: &impl serde::Serialize,
) -> std::io::Result<()> {
    if output.as_ref().is_none_or(|(current, _)| *current != hour) {
        *output = Some((
            hour,
            open(&directory.join(format!("diagnostics-{hour}.jsonl")))?,
        ));
    }
    // One append per complete JSONL line avoids interleaved serializer fragments.
    let mut line = serde_json::to_vec(record)?;
    line.push(b'\n');
    output.as_mut().expect("opened above").1.write_all(&line)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_lines_append_and_rotate_without_losing_records() {
        let directory = tempfile::tempdir().unwrap();
        let mut output = None;
        for (hour, id) in [(1, 1), (1, 2), (2, 3)] {
            append(
                &mut output,
                directory.path(),
                hour,
                &serde_json::json!({"id": id}),
            )
            .unwrap();
        }
        drop(output);
        let first = fs::read_to_string(directory.path().join("diagnostics-1.jsonl")).unwrap();
        let values: Vec<serde_json::Value> = first
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            values,
            vec![serde_json::json!({"id": 1}), serde_json::json!({"id": 2})]
        );
        let second = fs::read_to_string(directory.path().join("diagnostics-2.jsonl")).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(second.trim()).unwrap(),
            serde_json::json!({"id": 3})
        );
    }
}
