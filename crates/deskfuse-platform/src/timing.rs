use std::{
    collections::VecDeque,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Source of an input hitch observed during a diagnostic window.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitchSource {
    Mouse,
    Capture,
    Network,
    Receiver,
}

/// User-facing quality grade for a recent input-sharing window.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputQualityGrade {
    Idle,
    Smooth,
    Fair,
    Poor,
}

/// Bounded metadata-only quality aggregation. It deliberately stores counts and
/// timings, never coordinates, key values, clipboard data, or peer secrets.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InputQualityWindow {
    pub moving_seconds: u32,
    pub rtt_ms: Vec<f64>,
    pub hitches: Vec<(HitchSource, f64)>,
    pub slow_injections: u32,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InputQualitySnapshot {
    pub grade: InputQualityGrade,
    pub rtt_p50_ms: Option<f64>,
    pub rtt_p95_ms: Option<f64>,
    pub hitches_per_minute: f64,
    /// 网络路径（发送队列、丢包、拥塞）造成的卡顿率。
    pub hitches_network_per_minute: f64,
    /// 本机采集排队造成的卡顿率。
    pub hitches_capture_per_minute: f64,
    pub slow_injections: u32,
    /// 慢注入率，供图表按来源分色。
    pub slow_injections_per_minute: f64,
}

impl InputQualityWindow {
    pub const WINDOW_SECONDS: u32 = 60;

    pub fn observe_rtt(&mut self, rtt_ms: f64) {
        if rtt_ms.is_finite() && (0.0..=60_000.0).contains(&rtt_ms) {
            self.rtt_ms.push(rtt_ms);
            if self.rtt_ms.len() > 4096 {
                self.rtt_ms.drain(..self.rtt_ms.len() - 4096);
            }
        }
    }

    pub fn observe_motion_second(&mut self) {
        self.moving_seconds = self
            .moving_seconds
            .saturating_add(1)
            .min(Self::WINDOW_SECONDS);
    }

    pub fn observe_hitch(&mut self, source: HitchSource, delay_ms: f64) {
        if delay_ms.is_finite() && (0.0..=60_000.0).contains(&delay_ms) {
            self.hitches.push((source, delay_ms));
            if self.hitches.len() > 4096 {
                self.hitches.drain(..self.hitches.len() - 4096);
            }
        }
    }

    /// 记入已经发生的慢注入次数。阈值由注入侧按次判定，这里只累加。
    pub fn observe_slow_injections(&mut self, count: u32) {
        self.slow_injections = self.slow_injections.saturating_add(count);
    }

    pub fn snapshot(&self) -> InputQualitySnapshot {
        let mut rtt = self.rtt_ms.clone();
        rtt.sort_by(f64::total_cmp);
        let percentile =
            |q: f64| (!rtt.is_empty()).then(|| rtt[((rtt.len() - 1) as f64 * q).round() as usize]);
        let hitches = self.hitches.len() as u32;
        let minutes = f64::from(self.moving_seconds.max(30)) / 60.0;
        let rate = f64::from(hitches) / minutes;
        let p95 = percentile(0.95);
        let grade = if self.moving_seconds < 3 && hitches == 0 {
            InputQualityGrade::Idle
        } else if rate <= 3.0 && p95.unwrap_or(0.0) <= 20.0 {
            InputQualityGrade::Smooth
        } else if rate <= 10.0 && p95.unwrap_or(0.0) <= 50.0 {
            InputQualityGrade::Fair
        } else {
            InputQualityGrade::Poor
        };
        let per_minute = |count: u32| (f64::from(count) / minutes * 10.0).round() / 10.0;
        let source_count = |source: HitchSource| {
            self.hitches
                .iter()
                .filter(|(hitched, _)| *hitched == source)
                .count() as u32
        };
        InputQualitySnapshot {
            grade,
            rtt_p50_ms: percentile(0.50),
            rtt_p95_ms: p95,
            hitches_per_minute: per_minute(hitches),
            hitches_network_per_minute: per_minute(source_count(HitchSource::Network)),
            hitches_capture_per_minute: per_minute(source_count(HitchSource::Capture)),
            slow_injections: self.slow_injections,
            slow_injections_per_minute: per_minute(self.slow_injections),
        }
    }
}

// Bounded ring for diagnostics export. Sized for a multi-minute reproduction
// rather than a few seconds, because a choppy-motion report is exported after
// the fact and rate-limited stages would otherwise evict the interesting part.
const CAPACITY: usize = 4096;
static RECORDS: OnceLock<Mutex<VecDeque<InputTiming>>> = OnceLock::new();
static DROPPED: AtomicU64 = AtomicU64::new(0);
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, serde::Serialize)]
pub struct InputTiming {
    pub sequence: u64,
    timestamp_ms: u128,
    subsystem: &'static str,
    stage: &'static str,
    duration_us: u128,
    detail: String,
}

pub use deskfuse_core::debug_logging::enabled;

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
        sequence: SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1,
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
    snapshot_since(0)
}

/// Incremental persistence without cloning the whole diagnostic ring each time.
pub fn snapshot_since(sequence: u64) -> InputTimingSnapshot {
    let records = RECORDS
        .get()
        .map(|records| {
            records
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .filter(|record| record.sequence > sequence)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    InputTimingSnapshot {
        records,
        dropped_records: DROPPED.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
mod quality_tests {
    use super::*;

    #[test]
    fn incremental_snapshot_excludes_already_persisted_records() {
        let before = SEQUENCE.load(Ordering::Relaxed);
        record("persistence_test", Duration::ZERO, String::new());
        let snapshot = snapshot_since(before);
        assert!(
            snapshot
                .records
                .iter()
                .all(|record| record.sequence > before)
        );
        assert!(
            snapshot
                .records
                .iter()
                .any(|record| record.stage == "persistence_test")
        );
        let last = snapshot.records.last().unwrap().sequence;
        assert!(
            snapshot_since(last)
                .records
                .iter()
                .all(|record| record.sequence > last)
        );
    }

    #[test]
    fn steady_input_is_smooth_and_reports_percentiles() {
        let mut window = InputQualityWindow::default();
        for _ in 0..40 {
            window.observe_motion_second();
            window.observe_rtt(4.0);
        }
        let result = window.snapshot();
        assert_eq!(result.grade, InputQualityGrade::Smooth);
        assert_eq!(result.rtt_p50_ms, Some(4.0));
        assert_eq!(result.rtt_p95_ms, Some(4.0));
    }

    #[test]
    fn network_hitches_raise_fair_grade_without_exposing_payloads() {
        let mut window = InputQualityWindow::default();
        for _ in 0..60 {
            window.observe_motion_second();
        }
        for _ in 0..6 {
            window.observe_hitch(HitchSource::Network, 35.0);
        }
        window.observe_slow_injections(1);
        let result = window.snapshot();
        assert_eq!(result.grade, InputQualityGrade::Fair);
        assert_eq!(result.hitches_per_minute, 6.0);
        assert_eq!(result.slow_injections, 1);
    }

    #[test]
    fn hitch_sources_are_reported_per_minute() {
        let mut window = InputQualityWindow::default();
        for _ in 0..60 {
            window.observe_motion_second();
        }
        for _ in 0..4 {
            window.observe_hitch(HitchSource::Network, 30.0);
        }
        window.observe_hitch(HitchSource::Capture, 12.0);
        window.observe_hitch(HitchSource::Mouse, 1.0);
        window.observe_slow_injections(2);
        let result = window.snapshot();
        assert_eq!(result.hitches_per_minute, 6.0);
        assert_eq!(result.hitches_network_per_minute, 4.0);
        assert_eq!(result.hitches_capture_per_minute, 1.0);
        assert_eq!(result.slow_injections_per_minute, 2.0);
    }

    #[test]
    fn invalid_samples_are_ignored_and_history_is_bounded() {
        let mut window = InputQualityWindow::default();
        window.observe_rtt(f64::NAN);
        window.observe_hitch(HitchSource::Capture, -1.0);
        for _ in 0..5000 {
            window.observe_rtt(1.0);
        }
        assert_eq!(window.rtt_ms.len(), 4096);
        assert!(window.hitches.is_empty());
    }
}
