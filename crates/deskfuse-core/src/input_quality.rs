//! 固定窗口的输入阶段计数。只保存聚合值，不保存坐标或按键。

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// 一条采样在本机经历的阶段。呈现时间由平台单独声明，不在这里伪造。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputStage {
    /// 采集到进入发送队列。
    CaptureQueue,
    /// 编码后交给传输。
    Send,
    /// 接收端注入。
    Inject,
}

/// 一次 1 Hz 折叠后的链路观察。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QualityTick {
    /// 采样时刻，毫秒。
    pub timestamp_ms: u64,
    /// 当前路径 RTT。
    pub rtt_micros: u64,
    /// 这一秒内的卡顿次数。
    pub hitches: u32,
    /// 发送队列深度。
    pub pending_send: u32,
    /// 这一秒新增丢弃。
    pub dropped_delta: u64,
    /// 这一秒新增替换。
    pub replaced_delta: u64,
    /// 这一秒新增发送。
    pub sent_delta: u64,
    /// 采集排队耗时合计。
    pub capture_queue_micros: u64,
    /// 发送耗时合计。
    pub send_micros: u64,
    /// 注入耗时合计。
    pub inject_micros: u64,
    /// 采集样本数。
    pub capture_count: u64,
    /// 发送样本数。
    pub send_count: u64,
    /// 注入样本数。
    pub inject_count: u64,
    /// 这一秒内达到慢注入阈值的注入次数。
    pub slow_injections: u32,
}

/// 传输层每秒交来的累计计数。增量在这里计算。
#[derive(Clone, Copy, Debug)]
pub struct QualityObservation {
    /// 采样时刻，毫秒。
    pub timestamp_ms: u64,
    /// 当前 RTT。
    pub rtt_micros: u64,
    /// 这一秒的卡顿次数。
    pub hitches: u32,
    /// 当前发送队列深度。
    pub pending_send: u32,
    /// 累计发送。
    pub sent: u64,
    /// 累计丢弃。
    pub dropped: u64,
    /// 累计替换。
    pub replaced: u64,
}

const WINDOW: usize = 60;

/// 单次注入达到这个耗时就算一次慢注入，单位微秒。
pub const SLOW_INJECTION_MICROS: u64 = 8_000;

struct StageTotals {
    capture_us: AtomicU64,
    capture_n: AtomicU64,
    send_us: AtomicU64,
    send_n: AtomicU64,
    inject_us: AtomicU64,
    inject_n: AtomicU64,
    slow_injections: AtomicU64,
}

static STAGES: StageTotals = StageTotals {
    capture_us: AtomicU64::new(0),
    capture_n: AtomicU64::new(0),
    send_us: AtomicU64::new(0),
    send_n: AtomicU64::new(0),
    inject_us: AtomicU64::new(0),
    inject_n: AtomicU64::new(0),
    slow_injections: AtomicU64::new(0),
};
static TICKS: Mutex<VecDeque<QualityTick>> = Mutex::new(VecDeque::new());
static LAST_TOTALS: Mutex<Option<(u64, u64, u64)>> = Mutex::new(None);
static CAPTURE_READY: AtomicBool = AtomicBool::new(false);

/// 相对采集是否已经就绪。默认否，避免未启动采集时协商升级。
pub fn set_relative_motion_capture_ready(ready: bool) {
    CAPTURE_READY.store(ready, Ordering::Release);
}

/// 当前进程的相对采集是否就绪。
pub fn relative_motion_capture_ready() -> bool {
    CAPTURE_READY.load(Ordering::Acquire)
}

/// macOS 与 Windows 构建具备相对注入路径。其他平台保持绝对坐标。
pub fn relative_motion_inject_ready() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// 跨进程交接的阶段合计。不含坐标。旧协议省略时全部为零。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingStages {
    /// 采集排队耗时合计，微秒。
    pub capture_queue_micros: u64,
    /// 采集样本数。
    pub capture_count: u64,
    /// 发送耗时合计，微秒。
    pub send_micros: u64,
    /// 发送样本数。
    pub send_count: u64,
    /// 注入耗时合计，微秒。
    pub inject_micros: u64,
    /// 注入样本数。
    pub inject_count: u64,
    /// 达到慢注入阈值的注入次数。
    #[serde(default)]
    pub slow_injections: u32,
}

impl PendingStages {
    /// 所有计数都是零时不写进协议。
    pub fn is_empty(&self) -> bool {
        self.capture_queue_micros == 0
            && self.capture_count == 0
            && self.send_micros == 0
            && self.send_count == 0
            && self.inject_micros == 0
            && self.inject_count == 0
            && self.slow_injections == 0
    }
}

/// 累加一个阶段耗时。热路径只做原子加，不分配。
pub fn record_input_stage(stage: InputStage, micros: u64) {
    let (time, count) = match stage {
        InputStage::CaptureQueue => (&STAGES.capture_us, &STAGES.capture_n),
        InputStage::Send => (&STAGES.send_us, &STAGES.send_n),
        InputStage::Inject => (&STAGES.inject_us, &STAGES.inject_n),
    };
    time.fetch_add(micros, Ordering::Relaxed);
    count.fetch_add(1, Ordering::Relaxed);
}

/// 记录一次注入耗时。达到阈值就计入待交接的慢注入次数。
/// 与阶段合计分工不同：阶段合计用于算平均耗时，而平均值会把少数几次长卡顿
/// 摊平到看不见；这里要的是「有多少次真的卡了」。
pub fn note_injection_duration(micros: u64) {
    if micros >= SLOW_INJECTION_MICROS {
        STAGES.slow_injections.fetch_add(1, Ordering::Relaxed);
    }
}

/// 把自上次以来的阶段计数折进 60 秒窗口。
pub fn note_quality_tick(observation: QualityObservation) -> QualityTick {
    let stages = take_pending_stages();
    let (dropped_delta, replaced_delta, sent_delta) = {
        let Ok(mut last) = LAST_TOTALS.lock() else {
            add_pending_stages(stages);
            return QualityTick::default();
        };
        let current = (observation.dropped, observation.replaced, observation.sent);
        let delta = last.map_or((0, 0, 0), |previous| {
            (
                current.0.saturating_sub(previous.0),
                current.1.saturating_sub(previous.1),
                current.2.saturating_sub(previous.2),
            )
        });
        *last = Some(current);
        delta
    };
    let tick = QualityTick {
        timestamp_ms: observation.timestamp_ms,
        rtt_micros: observation.rtt_micros,
        hitches: observation.hitches,
        pending_send: observation.pending_send,
        dropped_delta,
        replaced_delta,
        sent_delta,
        capture_queue_micros: stages.capture_queue_micros,
        send_micros: stages.send_micros,
        inject_micros: stages.inject_micros,
        capture_count: stages.capture_count,
        send_count: stages.send_count,
        inject_count: stages.inject_count,
        slow_injections: stages.slow_injections,
    };
    let Ok(mut ticks) = TICKS.lock() else {
        add_pending_stages(stages);
        return QualityTick::default();
    };
    if ticks.len() == WINDOW {
        ticks.pop_front();
    }
    ticks.push_back(tick);
    tick
}

/// 取走本进程尚未折叠的阶段合计。调用方负责交给拥有报告的进程。
pub fn take_pending_stages() -> PendingStages {
    let (capture_queue_micros, capture_count) = take(&STAGES.capture_us, &STAGES.capture_n);
    let (send_micros, send_count) = take(&STAGES.send_us, &STAGES.send_n);
    let (inject_micros, inject_count) = take(&STAGES.inject_us, &STAGES.inject_n);
    let slow_injections = STAGES
        .slow_injections
        .swap(0, Ordering::Relaxed)
        .min(u64::from(u32::MAX)) as u32;
    PendingStages {
        capture_queue_micros,
        capture_count,
        send_micros,
        send_count,
        inject_micros,
        inject_count,
        slow_injections,
    }
}

/// 把另一进程交来的阶段合计加回本进程。空快照不碰原子计数。
pub fn add_pending_stages(stages: PendingStages) {
    if stages.is_empty() {
        return;
    }
    STAGES
        .capture_us
        .fetch_add(stages.capture_queue_micros, Ordering::Relaxed);
    STAGES
        .capture_n
        .fetch_add(stages.capture_count, Ordering::Relaxed);
    STAGES
        .send_us
        .fetch_add(stages.send_micros, Ordering::Relaxed);
    STAGES
        .send_n
        .fetch_add(stages.send_count, Ordering::Relaxed);
    STAGES
        .inject_us
        .fetch_add(stages.inject_micros, Ordering::Relaxed);
    STAGES
        .inject_n
        .fetch_add(stages.inject_count, Ordering::Relaxed);
    STAGES
        .slow_injections
        .fetch_add(u64::from(stages.slow_injections), Ordering::Relaxed);
}

/// 把尚未折叠的阶段耗时并进当前窗口，不改传输累计计数。
pub fn fold_pending_stages() {
    let stages = take_pending_stages();
    if stages.is_empty() {
        return;
    }
    let Ok(mut ticks) = TICKS.lock() else {
        add_pending_stages(stages);
        return;
    };
    if let Some(last) = ticks.back_mut() {
        last.capture_queue_micros = last
            .capture_queue_micros
            .saturating_add(stages.capture_queue_micros);
        last.capture_count = last.capture_count.saturating_add(stages.capture_count);
        last.send_micros = last.send_micros.saturating_add(stages.send_micros);
        last.send_count = last.send_count.saturating_add(stages.send_count);
        last.inject_micros = last.inject_micros.saturating_add(stages.inject_micros);
        last.inject_count = last.inject_count.saturating_add(stages.inject_count);
        last.slow_injections = last.slow_injections.saturating_add(stages.slow_injections);
        return;
    }
    ticks.push_back(QualityTick {
        capture_queue_micros: stages.capture_queue_micros,
        capture_count: stages.capture_count,
        send_micros: stages.send_micros,
        send_count: stages.send_count,
        inject_micros: stages.inject_micros,
        inject_count: stages.inject_count,
        slow_injections: stages.slow_injections,
        ..QualityTick::default()
    });
}

/// 复制当前窗口，供报告生成使用。调用方拿到的是快照。
pub fn quality_ticks() -> Vec<QualityTick> {
    TICKS
        .lock()
        .map(|ticks| ticks.iter().copied().collect())
        .unwrap_or_default()
}

fn take(time: &AtomicU64, count: &AtomicU64) -> (u64, u64) {
    (
        time.swap(0, Ordering::Relaxed),
        count.swap(0, Ordering::Relaxed),
    )
}

/// 设备计数是否能进入相对运动包。单位不是像素，也不是毫米。
pub fn relative_delta_ok(value: f64) -> bool {
    value.is_finite() && value.abs() <= 32768.0
}

/// 原始鼠标增量是否可用。绝对标志、零增量和超限增量都丢掉。
pub fn accept_raw_delta(absolute: bool, dx: i32, dy: i32) -> Option<(i32, i32)> {
    if absolute || (dx == 0 && dy == 0) {
        return None;
    }
    if relative_delta_ok(f64::from(dx)) && relative_delta_ok(f64::from(dy)) {
        Some((dx, dy))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_injections_travel_through_the_stage_handoff() {
        // 注入侧只把达到阈值的那次记成慢注入。
        note_injection_duration(SLOW_INJECTION_MICROS - 1);
        note_injection_duration(SLOW_INJECTION_MICROS);
        // 交接给拥有报告的进程，空快照会被跳过，所以这一次必须不为空。
        let stages = take_pending_stages();
        assert_eq!(stages.slow_injections, 1);
        assert!(!stages.is_empty());
        add_pending_stages(stages);
        let tick = note_quality_tick(QualityObservation {
            timestamp_ms: 0,
            rtt_micros: 0,
            hitches: 0,
            pending_send: 0,
            sent: 0,
            dropped: 0,
            replaced: 0,
        });
        assert_eq!(tick.slow_injections, 1);
    }

    #[test]
    fn raw_delta_rejects_absolute_zero_and_oversized_counts() {
        assert_eq!(accept_raw_delta(false, 3, -4), Some((3, -4)));
        assert_eq!(accept_raw_delta(true, 3, -4), None);
        assert_eq!(accept_raw_delta(false, 0, 0), None);
        assert_eq!(accept_raw_delta(false, 32769, 1), None);
        assert!(relative_delta_ok(-32768.0));
        assert!(!relative_delta_ok(f64::NAN));
        assert!(!relative_delta_ok(f64::INFINITY));
    }
}
