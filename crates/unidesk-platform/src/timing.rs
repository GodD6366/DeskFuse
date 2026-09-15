use std::{
    collections::VecDeque,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

// Bounded ring for diagnostics export. Sized for a multi-minute reproduction
// rather than a few seconds, because a choppy-motion report is exported after
// the fact and rate-limited stages would otherwise evict the interesting part.
const CAPACITY: usize = 4096;
static RECORDS: OnceLock<Mutex<VecDeque<InputTiming>>> = OnceLock::new();
static DROPPED: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, serde::Serialize)]
pub struct InputTiming {
    timestamp_ms: u128,
    subsystem: &'static str,
    stage: &'static str,
    duration_us: u128,
    detail: String,
}

pub use unidesk_core::debug_logging::enabled;

/// Gate every argument, including `format!` and `elapsed()`, before it is evaluated.
#[macro_export]
macro_rules! record_input_timing {
    ($stage:expr, $duration:expr, $detail:expr $(,)?) => {
        if $crate::timing::enabled() {
            $crate::timing::record($stage, $duration, $detail);
        }
    };
}
pub use crate::record_input_timing as record;

pub fn record(stage: &'static str, duration: Duration, detail: String) {
    if !enabled() {
        return;
    }
    let records = RECORDS.get_or_init(|| Mutex::new(VecDeque::with_capacity(CAPACITY)));
    let Ok(mut records) = records.try_lock() else {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if records.len() == CAPACITY {
        records.pop_front();
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
    records.push_back(InputTiming {
        timestamp_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        subsystem: "input_timing",
        stage,
        duration_us: duration.as_micros(),
        detail,
    });
}

#[derive(serde::Serialize)]
pub struct InputTimingSnapshot {
    pub records: Vec<InputTiming>,
    pub dropped_records: u64,
}

pub fn snapshot() -> InputTimingSnapshot {
    let records = RECORDS
        .get()
        .map(|records| {
            records
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    InputTimingSnapshot {
        records,
        dropped_records: DROPPED.load(Ordering::Relaxed),
    }
}
