#![allow(dead_code)]
#![allow(clippy::too_many_arguments)]

use serde::Serialize;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(target_os = "macos")]
pub fn install_panic_recorder() {
    use std::backtrace::Backtrace;
    use std::fs::{DirBuilder, OpenOptions};
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    use std::sync::atomic::{AtomicBool, Ordering};

    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let directory = std::path::PathBuf::from(home).join("Library/Logs/uniDesk");
    if DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)
        .is_err()
    {
        eprintln!("uniDesk: panic record directory unavailable");
        return;
    }
    let path = directory.join("panic-first.log");
    let recorded = AtomicBool::new(false);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |information| {
        if !recorded.swap(true, Ordering::SeqCst)
            && let Ok(mut file) = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .mode(0o600)
                .open(&path)
        {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let thread = std::thread::current();
            let _ = writeln!(
                file,
                "timestamp_ms={timestamp} pid={} version={} thread={}",
                std::process::id(),
                env!("CARGO_PKG_VERSION"),
                thread.name().unwrap_or("unnamed")
            );
            if let Some(location) = information.location() {
                let _ = writeln!(
                    file,
                    "source={}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                );
            }
            let _ = file.sync_all();
            let _ = writeln!(file, "{}", Backtrace::force_capture());
            let _ = file.sync_all();
        }
        previous(information);
    }));
}

/// A deliberately small set of levels for records that are safe to show in the
/// settings UI and safe to export for support diagnostics.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Info,
    Warning,
    Error,
}

impl DiagnosticLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DiagnosticRecord {
    pub sequence: u64,
    pub timestamp: String,
    pub level: DiagnosticLevel,
    pub subsystem: String,
    pub event: String,
    pub request_id: Option<String>,
    pub duration_ms: Option<u64>,
    pub result: Option<String>,
    pub error_code: Option<String>,
}

#[derive(Debug)]
struct DiagnosticStore {
    capacity: usize,
    next_sequence: u64,
    records: VecDeque<DiagnosticRecord>,
}

/// Thread-safe, bounded diagnostics. It intentionally contains metadata only;
/// callers must not put clipboard bodies, window contents, or permission data
/// into records.
#[derive(Clone, Debug)]
pub struct Diagnostics {
    inner: Arc<Mutex<DiagnosticStore>>,
}

impl Diagnostics {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            inner: Arc::new(Mutex::new(DiagnosticStore {
                capacity,
                next_sequence: 1,
                records: VecDeque::with_capacity(capacity),
            })),
        }
    }

    pub fn record(
        &self,
        level: DiagnosticLevel,
        subsystem: impl Into<String>,
        event: impl Into<String>,
        result: Option<String>,
        error_code: Option<String>,
    ) -> DiagnosticRecord {
        self.record_with_metadata(level, subsystem, event, None, None, result, error_code)
    }

    pub fn record_with_metadata(
        &self,
        level: DiagnosticLevel,
        subsystem: impl Into<String>,
        event: impl Into<String>,
        request_id: Option<String>,
        duration_ms: Option<u64>,
        result: Option<String>,
        error_code: Option<String>,
    ) -> DiagnosticRecord {
        let mut store = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = DiagnosticRecord {
            sequence: store.next_sequence,
            timestamp: timestamp_now(),
            level,
            subsystem: subsystem.into(),
            event: event.into(),
            request_id,
            duration_ms,
            result,
            error_code,
        };
        store.next_sequence = store.next_sequence.saturating_add(1);
        store.records.push_back(record.clone());
        while store.records.len() > store.capacity {
            store.records.pop_front();
        }
        record
    }

    pub fn snapshot(&self) -> Vec<DiagnosticRecord> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .records
            .iter()
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .records
            .len()
    }

    pub fn to_json_lines(&self) -> String {
        self.snapshot()
            .into_iter()
            .filter_map(|record| serde_json::to_string(&record).ok())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::new(128)
    }
}

pub fn timestamp_now() -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}Z", elapsed.as_secs(), elapsed.subsec_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_are_bounded_and_json_lines_are_exportable() {
        let diagnostics = Diagnostics::new(2);
        diagnostics.record(DiagnosticLevel::Info, "test", "one", None, None);
        diagnostics.record(DiagnosticLevel::Warning, "test", "two", None, None);
        diagnostics.record(
            DiagnosticLevel::Error,
            "test",
            "three",
            None,
            Some("E_TEST".into()),
        );

        let snapshot = diagnostics.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].event, "two");
        assert_eq!(snapshot[1].error_code.as_deref(), Some("E_TEST"));
        assert_eq!(diagnostics.to_json_lines().lines().count(), 2);
    }
}
