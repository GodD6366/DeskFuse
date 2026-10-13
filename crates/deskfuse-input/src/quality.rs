//! 指针模式协商、只读物理归一化、质量报告、建议和历史对比。
//!
//! 默认不改注入结果。诊断只保留固定窗口里的聚合值。

use std::collections::VecDeque;
use std::sync::Mutex;

use deskfuse_core::{MotionPointerMode, QualityTick};
use serde::{Deserialize, Serialize};

/// 自动物理归一化仍关闭。报告里的诊断位移只用于对比。
pub const NORMALIZATION_ENABLED: bool = false;
/// 会话结束时保留的报告份数。
pub const QUALITY_HISTORY_CAPACITY: usize = 16;

/// 接收方做能力决定时需要的事实。函数本身不读全局状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointerNegotiation {
    /// 双方都能走 datagram。
    pub datagram_available: bool,
    /// 协商结果里有相对移动能力。
    pub peer_relative: bool,
    /// 控制端声明原始采集已就绪。
    pub request_relative_ready: bool,
    /// 本机可以做相对注入。
    pub inject_ready: bool,
}

/// 一次租约固定下来的指针决定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointerDecision {
    /// 租约内不变的模式。
    pub mode: MotionPointerMode,
    /// datagram 是否启用。可靠模式为否。
    pub motion_enabled: bool,
    /// 对外的模式名：`relative`、`hybrid` 或 `reliable-only`。
    pub motion_mode: &'static str,
    /// 没有进入相对模式时给用户看的原因。
    pub fallback: Option<&'static str>,
}

/// 按能力、采集和注入条件选择模式。不满足相对条件时留在绝对坐标或可靠路径。
pub fn negotiate_pointer(input: PointerNegotiation) -> PointerDecision {
    if input.datagram_available
        && input.peer_relative
        && input.request_relative_ready
        && input.inject_ready
    {
        return PointerDecision {
            mode: MotionPointerMode::Relative,
            motion_enabled: true,
            motion_mode: "relative",
            fallback: None,
        };
    }
    if input.datagram_available {
        let fallback = if !input.peer_relative {
            Some("对端未声明相对移动能力，已使用绝对坐标")
        } else if !input.request_relative_ready {
            Some("本机原始相对采集未就绪，已使用绝对坐标")
        } else {
            Some("对端相对注入未就绪，已使用绝对坐标")
        };
        return PointerDecision {
            mode: MotionPointerMode::Absolute,
            motion_enabled: true,
            motion_mode: "hybrid",
            fallback,
        };
    }
    PointerDecision {
        mode: MotionPointerMode::Reliable,
        motion_enabled: false,
        motion_mode: "reliable-only",
        fallback: None,
    }
}

/// 归一化使用的显示器几何。负坐标原点不影响增量。
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayGeometry {
    /// 逻辑宽度。
    pub logical_width: f64,
    /// 逻辑高度。
    pub logical_height: f64,
    /// 物理像素 / 逻辑点。
    pub scale: f64,
    /// 面板物理宽度，毫米。
    pub physical_width_mm: Option<f64>,
    /// 面板物理高度，毫米。
    pub physical_height_mm: Option<f64>,
    /// 0、90、180 或 270。
    pub rotation_degrees: u16,
    /// 逻辑原点，可以为负。
    pub frame_x: f64,
    /// 逻辑原点，可以为负。
    pub frame_y: f64,
    /// 拓扑世代。热插拔后由调用方换成新几何。
    pub topology_generation: u64,
}

/// 只读归一化结果。`logical_*` 在关闭自动归一化时不改变调用方要注入的增量。
#[derive(Clone, Debug, PartialEq)]
pub struct NormalizedMotion {
    /// 关闭自动归一化时等于旋转并乘灵敏度后的增量。
    pub logical_dx: f64,
    /// 关闭自动归一化时等于旋转并乘灵敏度后的增量。
    pub logical_dy: f64,
    /// 如果启用物理归一化，注入端会使用的增量。
    pub diagnostic_dx: f64,
    /// 如果启用物理归一化，注入端会使用的增量。
    pub diagnostic_dy: f64,
    /// 源增量对应的毫米。尺寸无效时为空。
    pub millimeters_dx: Option<f64>,
    /// 源增量对应的毫米。尺寸无效时为空。
    pub millimeters_dy: Option<f64>,
    /// 当前是否已经按物理尺寸改写注入。
    pub used_physical: bool,
    /// 没有采用物理尺寸的原因。
    pub reason: Option<&'static str>,
}

/// 把源显示器上的增量换成目标显示器上的等毫米诊断位移。
pub fn normalize_pointer(
    source: &DisplayGeometry,
    target: &DisplayGeometry,
    dx: f64,
    dy: f64,
    sensitivity: f64,
) -> NormalizedMotion {
    let (rotated_x, rotated_y) = rotate(dx, dy, source.rotation_degrees);
    let scaled_x = rotated_x * finite_or(sensitivity, 1.0);
    let scaled_y = rotated_y * finite_or(sensitivity, 1.0);
    let Some((source_mm_x, source_mm_y)) = millimeters(source, rotated_x, rotated_y) else {
        return NormalizedMotion {
            logical_dx: scaled_x,
            logical_dy: scaled_y,
            diagnostic_dx: scaled_x,
            diagnostic_dy: scaled_y,
            millimeters_dx: None,
            millimeters_dy: None,
            used_physical: false,
            reason: Some("physical_size_missing"),
        };
    };
    let millimeters_dx = Some(source_mm_x * finite_or(sensitivity, 1.0));
    let millimeters_dy = Some(source_mm_y * finite_or(sensitivity, 1.0));
    let diagnostic = if let Some((pixel_x, pixel_y)) =
        pixels_from_mm(target, source_mm_x * finite_or(sensitivity, 1.0), source_mm_y * finite_or(sensitivity, 1.0))
    {
        let (dx, dy) = rotate(pixel_x, pixel_y, inverse_rotation(target.rotation_degrees));
        (dx, dy)
    } else {
        (scaled_x, scaled_y)
    };
    // 原点只用于确认调用方把几何传进来了，不参与增量。
    let _ = (source.frame_x, source.frame_y, source.topology_generation, target.frame_x, target.frame_y, target.topology_generation);
    NormalizedMotion {
        logical_dx: scaled_x,
        logical_dy: scaled_y,
        diagnostic_dx: diagnostic.0,
        diagnostic_dy: diagnostic.1,
        millimeters_dx,
        millimeters_dy,
        used_physical: NORMALIZATION_ENABLED,
        reason: if NORMALIZATION_ENABLED {
            None
        } else {
            Some("normalization_disabled")
        },
    }
}

/// 曲线上的一个点。没有坐标。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QualityCurvePoint {
    /// 窗口内的秒序号。
    pub second: u32,
    /// RTT，毫秒。
    pub rtt_ms: f64,
    /// 采集、发送、RTT 与注入相加的估计，毫秒。
    pub e2e_ms: f64,
}

/// 一条可执行、可复测的建议。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QualitySuggestion {
    /// 稳定标识，复测时用它对照。
    pub id: String,
    /// `act` 需要处理、`info` 只是说明、`ok` 无需动作。
    pub priority: String,
    /// 触发这条建议的指标。
    pub trigger: String,
    /// 用户可以做的动作。
    pub action: String,
    /// 期望改善的指标。
    pub expected: String,
    /// 操作后如何再测。
    pub retest: String,
}

/// 最近 60 秒的质量报告。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InputQualityReport {
    /// `idle`、`smooth`、`fair` 或 `poor`。
    pub grade: String,
    /// 当前指针模式名。
    pub mode: String,
    /// 回退原因。
    pub fallback: Option<String>,
    /// RTT P50，微秒。
    pub rtt_p50_micros: u64,
    /// RTT P95，微秒。
    pub rtt_p95_micros: u64,
    /// RTT P99，微秒。
    pub rtt_p99_micros: u64,
    /// 端到端估计 P50，微秒。
    pub e2e_p50_micros: u64,
    /// 端到端估计 P95，微秒。
    pub e2e_p95_micros: u64,
    /// 窗口内卡顿次数。
    pub hitches: u32,
    /// 丢包率。
    pub loss_rate: f64,
    /// 替换率。
    pub replace_rate: f64,
    /// 观察到的最大发送队列深度。
    pub queue_depth: u32,
    /// 平均注入耗时，微秒。
    pub inject_micros: u64,
    /// 平台没有光标呈现时间。
    pub present_supported: bool,
    /// 曲线点。
    pub samples: Vec<QualityCurvePoint>,
    /// 按原因生成的建议。
    pub suggestions: Vec<QualitySuggestion>,
    /// 窗口里还没有样本。
    pub empty: bool,
}

impl InputQualityReport {
    /// 空报告不写进状态 JSON。
    pub fn is_idle(&self) -> bool {
        self.empty
    }
}

/// 用一份采样切片生成报告。测试可以传入自己的切片，不碰进程全局窗口。
pub fn build_report(ticks: &[QualityTick], mode: &str, fallback: Option<&str>) -> InputQualityReport {
    if ticks.is_empty() {
        let mut report = InputQualityReport {
            grade: "idle".into(),
            mode: mode.to_owned(),
            fallback: fallback.map(str::to_owned),
            present_supported: false,
            empty: true,
            ..InputQualityReport::default()
        };
        report.suggestions = suggestions(&report);
        return report;
    }
    let mut rtt = ticks.iter().map(|tick| tick.rtt_micros).collect::<Vec<_>>();
    let mut e2e = ticks.iter().map(e2e_micros).collect::<Vec<_>>();
    rtt.sort_unstable();
    e2e.sort_unstable();
    let sent: u64 = ticks.iter().map(|tick| tick.sent_delta).sum();
    let dropped: u64 = ticks.iter().map(|tick| tick.dropped_delta).sum();
    let replaced: u64 = ticks.iter().map(|tick| tick.replaced_delta).sum();
    let hitches: u32 = ticks.iter().map(|tick| tick.hitches).fold(0, u32::saturating_add);
    let inject_count: u64 = ticks.iter().map(|tick| tick.inject_count).sum();
    let inject_us: u64 = ticks.iter().map(|tick| tick.inject_micros).sum();
    let minutes = (ticks.len() as f64 / 60.0).max(0.5);
    let hitch_rate = f64::from(hitches) / minutes;
    let p95_ms = percentile(&rtt, 0.95) as f64 / 1000.0;
    let grade = if ticks.len() < 3 && hitches == 0 {
        "idle"
    } else if hitch_rate <= 3.0 && p95_ms <= 20.0 {
        "smooth"
    } else if hitch_rate <= 10.0 && p95_ms <= 50.0 {
        "fair"
    } else {
        "poor"
    };
    let mut report = InputQualityReport {
        grade: grade.into(),
        mode: mode.to_owned(),
        fallback: fallback.map(str::to_owned),
        rtt_p50_micros: percentile(&rtt, 0.50),
        rtt_p95_micros: percentile(&rtt, 0.95),
        rtt_p99_micros: percentile(&rtt, 0.99),
        e2e_p50_micros: percentile(&e2e, 0.50),
        e2e_p95_micros: percentile(&e2e, 0.95),
        hitches,
        loss_rate: ratio(dropped, sent.saturating_add(dropped)),
        replace_rate: ratio(replaced, sent.saturating_add(replaced)),
        queue_depth: ticks.iter().map(|tick| tick.pending_send).max().unwrap_or(0),
        inject_micros: inject_us.checked_div(inject_count).unwrap_or(0),
        present_supported: false,
        samples: ticks
            .iter()
            .enumerate()
            .map(|(index, tick)| QualityCurvePoint {
                second: index as u32,
                rtt_ms: tick.rtt_micros as f64 / 1000.0,
                e2e_ms: e2e_micros(tick) as f64 / 1000.0,
            })
            .collect(),
        suggestions: Vec::new(),
        empty: false,
    };
    report.suggestions = suggestions(&report);
    report
}

/// 读取进程内 60 秒窗口。模式和回退原因由调用方从租约填入。
pub fn current_report(mode: &str, fallback: Option<&str>) -> InputQualityReport {
    build_report(&deskfuse_core::quality_ticks(), mode, fallback)
}

/// 按瓶颈生成建议。呈现时间平台不支持，因此不把它当成抖动证据。
pub fn suggestions(report: &InputQualityReport) -> Vec<QualitySuggestion> {
    let mut items = Vec::new();
    if report.fallback.as_ref().is_some_and(|reason| !reason.is_empty()) {
        items.push(suggestion(
            "act",
            "relative-unavailable",
            format!("回退原因：{}", report.fallback.as_deref().unwrap_or_default()),
            "保持当前绝对坐标或可靠路径。对端升级、补齐原始采集或注入权限后再协商相对移动。",
            "下一次租约显示相对移动，或回退原因消失。",
            "重新建立控制会话后看模式和回退原因。",
        ));
    }
    if report.empty {
        return items;
    }
    if report.rtt_p95_micros > 50_000 && report.loss_rate < 0.02 {
        items.push(suggestion(
            "act",
            "rtt",
            format!("P95 RTT {:.1} ms，丢包率 {:.1}%", report.rtt_p95_micros as f64 / 1000.0, report.loss_rate * 100.0),
            "缩短 Wi-Fi 距离，避开拥塞信道，或改用有线连接。",
            "P95 RTT 下降。",
            "保持移动鼠标一分钟后再次导出诊断。",
        ));
    }
    if report.loss_rate >= 0.02 || report.replace_rate >= 0.05 {
        items.push(suggestion(
            "act",
            "loss",
            format!("丢包率 {:.1}%，替换率 {:.1}%", report.loss_rate * 100.0, report.replace_rate * 100.0),
            "优先使用有线网络，并检查无线干扰和接收端负载。",
            "丢包率和替换率下降。",
            "同一段移动路径上再采一分钟。",
        ));
    }
    if report.queue_depth >= 8 {
        items.push(suggestion(
            "act",
            "queue",
            format!("发送队列深度 {}", report.queue_depth),
            "降低其他进程的 CPU 占用，确认接收端事件循环没有被拖住。",
            "队列深度回到个位数以下。",
            "观察下一分钟的队列深度峰值。",
        ));
    }
    if report.inject_micros >= 8_000 {
        items.push(suggestion(
            "act",
            "inject",
            format!("平均注入耗时 {:.1} ms", report.inject_micros as f64 / 1000.0),
            "检查辅助功能或输入监控权限，并降低本机负载。",
            "注入耗时降到 8 ms 以下。",
            "权限恢复后重新采样一分钟。",
        ));
    }
    if !report.present_supported {
        items.push(suggestion(
            "info",
            "present",
            "平台没有提供光标呈现时间。".into(),
            "呈现抖动不能从这条报告单独判断。先看网络、队列和注入。",
            "其余瓶颈下降后，光标跟随主观变稳。",
            "处理上面的建议后复测，不要把缺失的呈现时间当成故障。",
        ));
    }
    items
}

/// 操作前后的指标差。
#[derive(Clone, Debug, PartialEq)]
pub struct QualityComparison {
    /// P95 RTT 变化，微秒。负值表示变快。
    pub rtt_p95_delta_micros: i64,
    /// 端到端 P95 变化，微秒。
    pub e2e_p95_delta_micros: i64,
    /// 卡顿次数变化。
    pub hitch_delta: i64,
    /// 主要瓶颈是否改善。
    pub improved: bool,
    /// 给人读的结论。
    pub note: String,
}

/// 对比两份报告，不读取全局历史。
pub fn compare(before: &InputQualityReport, after: &InputQualityReport) -> QualityComparison {
    let rtt_p95_delta_micros = after.rtt_p95_micros as i64 - before.rtt_p95_micros as i64;
    let e2e_p95_delta_micros = after.e2e_p95_micros as i64 - before.e2e_p95_micros as i64;
    let hitch_delta = after.hitches as i64 - before.hitches as i64;
    let improved = rtt_p95_delta_micros < 0 || e2e_p95_delta_micros < 0 || hitch_delta < 0;
    let note = if improved {
        "复测指标下降。".to_owned()
    } else {
        "复测指标没有下降。".to_owned()
    };
    QualityComparison {
        rtt_p95_delta_micros,
        e2e_p95_delta_micros,
        hitch_delta,
        improved,
        note,
    }
}

/// 按建议标识检查操作后的对应指标。
#[derive(Clone, Debug, PartialEq)]
pub struct RetestResult {
    /// 建议标识。
    pub suggestion_id: String,
    /// 目标指标是否改善。
    pub improved: bool,
    /// 给人读的结论。
    pub note: String,
}

/// 复测一条建议。未知标识不会被算成通过。
pub fn retest(suggestion_id: &str, before: &InputQualityReport, after: &InputQualityReport) -> RetestResult {
    let (improved, note) = match suggestion_id {
        "rtt" => (after.rtt_p95_micros < before.rtt_p95_micros, "对比 P95 RTT"),
        "loss" => (after.loss_rate < before.loss_rate, "对比丢包率"),
        "queue" => (after.queue_depth < before.queue_depth, "对比队列深度"),
        "inject" => (after.inject_micros < before.inject_micros, "对比注入耗时"),
        "relative-unavailable" => (
            after.mode == "relative" || after.fallback.is_none(),
            "对比模式和回退原因",
        ),
        _ => (false, "没有这条建议的复测规则"),
    };
    RetestResult {
        suggestion_id: suggestion_id.to_owned(),
        improved,
        note: format!("{note}：{}", if improved { "已改善" } else { "未改善" }),
    }
}

/// 固定容量的报告历史。测试持有自己的实例。
#[derive(Clone, Debug, Default)]
pub struct QualityHistory {
    entries: VecDeque<InputQualityReport>,
}

impl QualityHistory {
    /// 新的历史。
    pub fn new() -> Self {
        Self::default()
    }

    /// 归档一份报告，超过容量时丢掉最旧的。
    pub fn archive(&mut self, report: InputQualityReport) {
        if self.entries.len() == QUALITY_HISTORY_CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back(report);
    }

    /// 从旧到新的报告。
    pub fn entries(&self) -> impl Iterator<Item = &InputQualityReport> {
        self.entries.iter()
    }

    /// 对比最后两份。不足两份时为空。
    pub fn compare_latest(&self) -> Option<QualityComparison> {
        let after = self.entries.back()?;
        let before = self.entries.iter().rev().nth(1)?;
        Some(compare(before, after))
    }
}

static HISTORY: Mutex<QualityHistory> = Mutex::new(QualityHistory {
    entries: VecDeque::new(),
});
static ACTIVE_SESSION: Mutex<Option<String>> = Mutex::new(None);

/// 记录当前控制会话。诊断仍是进程级 60 秒窗口。
pub fn begin_session(session_id: &str) {
    let Ok(mut active) = ACTIVE_SESSION.lock() else {
        return;
    };
    if active.as_deref() == Some(session_id) {
        return;
    }
    let previous = active.replace(session_id.to_owned());
    drop(active);
    if previous.is_some()
        && let Ok(mut history) = HISTORY.lock()
    {
        history.archive(current_report("", None));
    }
}

/// 会话结束时把当时的报告放进固定历史。
pub fn archive_session(session_id: &str) {
    let matches = ACTIVE_SESSION
        .lock()
        .ok()
        .is_some_and(|mut active| active.take().as_deref() == Some(session_id));
    if !matches {
        return;
    }
    if let Ok(mut history) = HISTORY.lock() {
        history.archive(current_report("", None));
    }
}

/// 进程内历史快照。
pub fn history_snapshot() -> Vec<InputQualityReport> {
    HISTORY
        .lock()
        .map(|history| history.entries.iter().cloned().collect())
        .unwrap_or_default()
}

fn suggestion(
    priority: &str,
    id: &str,
    trigger: String,
    action: &str,
    expected: &str,
    retest: &str,
) -> QualitySuggestion {
    QualitySuggestion {
        id: id.to_owned(),
        priority: priority.to_owned(),
        trigger,
        action: action.to_owned(),
        expected: expected.to_owned(),
        retest: retest.to_owned(),
    }
}

fn percentile(sorted: &[u64], quantile: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() - 1) as f64 * quantile).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn ratio(part: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        part as f64 / total as f64
    }
}

fn e2e_micros(tick: &QualityTick) -> u64 {
    let capture = average(tick.capture_queue_micros, tick.capture_count);
    let send = average(tick.send_micros, tick.send_count);
    let inject = average(tick.inject_micros, tick.inject_count);
    capture
        .saturating_add(send)
        .saturating_add(tick.rtt_micros)
        .saturating_add(inject)
}

fn average(total: u64, count: u64) -> u64 {
    total.checked_div(count).unwrap_or(0)
}

fn rotate(dx: f64, dy: f64, degrees: u16) -> (f64, f64) {
    match degrees % 360 {
        90 => (-dy, dx),
        180 => (-dx, -dy),
        270 => (dy, -dx),
        _ => (dx, dy),
    }
}

fn inverse_rotation(degrees: u16) -> u16 {
    match degrees % 360 {
        90 => 270,
        180 => 180,
        270 => 90,
        _ => 0,
    }
}

fn millimeters(display: &DisplayGeometry, dx: f64, dy: f64) -> Option<(f64, f64)> {
    let width = display.physical_width_mm?;
    let height = display.physical_height_mm?;
    if !usable(width) || !usable(height) || !usable(display.logical_width) || !usable(display.logical_height) || !usable(display.scale) {
        return None;
    }
    Some((dx * width / display.logical_width, dy * height / display.logical_height))
}

fn pixels_from_mm(display: &DisplayGeometry, mm_x: f64, mm_y: f64) -> Option<(f64, f64)> {
    let width = display.physical_width_mm?;
    let height = display.physical_height_mm?;
    if !usable(width) || !usable(height) || !usable(display.logical_width) || !usable(display.logical_height) {
        return None;
    }
    Some((mm_x / width * display.logical_width, mm_y / height * display.logical_height))
}

fn usable(value: f64) -> bool {
    value.is_finite() && value > 0.0 && value < 10_000.0
}

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() && value > 0.0 { value } else { fallback }
}

/// 一行诊断 JSON。调用方先折叠阶段计数。对比不走 `Serialize`，避免把它写进状态协议。
pub fn diagnostics_record(process: &str, mode: &str, fallback: Option<&str>) -> String {
    let history = history_snapshot();
    let comparison = if history.len() >= 2 {
        let compared = compare(&history[history.len() - 2], &history[history.len() - 1]);
        Some(serde_json::json!({
            "rtt_p95_delta_micros": compared.rtt_p95_delta_micros,
            "e2e_p95_delta_micros": compared.e2e_p95_delta_micros,
            "hitch_delta": compared.hitch_delta,
            "improved": compared.improved,
            "note": compared.note,
        }))
    } else {
        None
    };
    serde_json::json!({
        "subsystem": "input_quality",
        "process": process,
        "report": current_report(mode, fallback),
        "history": history,
        "comparison": comparison,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(width: f64, height: f64, scale: f64, mm: Option<(f64, f64)>, rotation: u16, x: f64) -> DisplayGeometry {
        DisplayGeometry {
            logical_width: width,
            logical_height: height,
            scale,
            physical_width_mm: mm.map(|value| value.0),
            physical_height_mm: mm.map(|value| value.1),
            rotation_degrees: rotation,
            frame_x: x,
            frame_y: 0.0,
            topology_generation: 1,
        }
    }

    #[test]
    fn negotiation_keeps_absolute_until_every_relative_condition_holds() {
        let absolute = negotiate_pointer(PointerNegotiation {
            datagram_available: true,
            peer_relative: false,
            request_relative_ready: true,
            inject_ready: true,
        });
        assert_eq!(absolute.mode, MotionPointerMode::Absolute);
        assert_eq!(absolute.motion_mode, "hybrid");
        assert_eq!(absolute.fallback, Some("对端未声明相对移动能力，已使用绝对坐标"));
        let relative = negotiate_pointer(PointerNegotiation {
            datagram_available: true,
            peer_relative: true,
            request_relative_ready: true,
            inject_ready: true,
        });
        assert_eq!(relative.motion_mode, "relative");
        assert!(relative.fallback.is_none());
        let reliable = negotiate_pointer(PointerNegotiation {
            datagram_available: false,
            peer_relative: true,
            request_relative_ready: true,
            inject_ready: true,
        });
        assert_eq!(reliable.motion_mode, "reliable-only");
        assert!(!reliable.motion_enabled);
    }

    #[test]
    fn normalization_is_read_only_and_covers_mixed_geometry() {
        let source = geometry(1000.0, 500.0, 2.0, Some((300.0, 150.0)), 0, -1920.0);
        let target = geometry(2000.0, 1000.0, 1.0, Some((600.0, 300.0)), 90, 0.0);
        let motion = normalize_pointer(&source, &target, 10.0, 0.0, 1.0);
        assert!(!motion.used_physical);
        assert_eq!(motion.reason, Some("normalization_disabled"));
        assert_eq!(motion.logical_dx, 10.0);
        assert_eq!(motion.millimeters_dx, Some(3.0));
        let mut moved = geometry(1000.0, 500.0, 2.0, Some((300.0, 150.0)), 0, -4000.0);
        moved.topology_generation = 9;
        let again = normalize_pointer(&moved, &target, 10.0, 0.0, 1.0);
        assert_eq!(again.diagnostic_dx, motion.diagnostic_dx);
        assert_eq!(again.diagnostic_dy, motion.diagnostic_dy);
        let portrait = normalize_pointer(&geometry(1000.0, 500.0, 2.0, Some((300.0, 150.0)), 90, 0.0), &source, 4.0, 6.0, 1.0);
        assert_eq!(portrait.logical_dx, -6.0);
        assert_eq!(portrait.logical_dy, 4.0);
        let missing = normalize_pointer(&geometry(1000.0, 500.0, 2.0, None, 0, 0.0), &source, 8.0, 2.0, 1.0);
        assert_eq!(missing.reason, Some("physical_size_missing"));
        assert!(missing.millimeters_dx.is_none());
        let retina = geometry(1000.0, 1000.0, 2.0, Some((300.0, 300.0)), 0, 0.0);
        let plain = geometry(1000.0, 1000.0, 1.0, Some((600.0, 600.0)), 0, 0.0);
        let same_logical = normalize_pointer(&retina, &plain, 100.0, 0.0, 1.0);
        assert!((same_logical.diagnostic_dx - 50.0).abs() < 0.001);
    }

    #[test]
    fn suggestions_name_the_bottleneck_and_retest_checks_that_metric() {
        let tick = QualityTick {
            rtt_micros: 80_000,
            hitches: 1,
            pending_send: 12,
            // 丢包率要留在 2% 以下：一旦超过，命中的是丢包建议，
            // 而纯延迟建议本来就只在延迟不是丢包造成的时候才给。
            dropped_delta: 0,
            replaced_delta: 1,
            sent_delta: 20,
            inject_micros: 20_000,
            inject_count: 1,
            ..QualityTick::default()
        };
        let before = build_report(std::slice::from_ref(&tick), "hybrid", Some("对端未声明相对移动能力，已使用绝对坐标"));
        assert!(before.suggestions.iter().any(|item| item.id == "rtt" && !item.action.is_empty() && !item.retest.is_empty()));
        // 只有说明性建议是 info，其余都要求用户处理。
        assert!(
            before
                .suggestions
                .iter()
                .all(|item| item.priority == if item.id == "present" { "info" } else { "act" })
        );
        assert!(!before.present_supported);
        let mut after = before.clone();
        after.rtt_p95_micros = 10_000;
        after.loss_rate = 0.0;
        after.queue_depth = 1;
        after.inject_micros = 100;
        after.mode = "relative".into();
        after.fallback = None;
        assert!(retest("rtt", &before, &after).improved);
        assert!(retest("relative-unavailable", &before, &after).improved);
        assert!(!retest("unknown", &before, &after).improved);
        let mut history = QualityHistory::new();
        history.archive(before);
        history.archive(after);
        assert!(history.compare_latest().unwrap().improved);
        for index in 0..20 {
            history.archive(InputQualityReport { grade: index.to_string(), ..InputQualityReport::default() });
        }
        assert_eq!(history.entries().count(), QUALITY_HISTORY_CAPACITY);
    }
}
