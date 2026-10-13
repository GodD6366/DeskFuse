use std::collections::BTreeSet;

use crate::{DisplayId, EdgeMapping};
use crate::{
    Edge, InputEvent, KeyCode, Modifiers, Point, PointerSample, Timestamp, Topology, Vector2,
};
use serde::{Deserialize, Serialize};

/// Whether routing can currently cross the network.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    /// Input can be routed to a controlled peer.
    #[default]
    Connected,
    /// Input must remain local until a new connection is established.
    Disconnected,
}

/// Why control was returned to the local display.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReturnReason {
    /// The connection to the controlled peer was lost.
    Disconnected,
    /// The configured escape hotkey was pressed.
    EscapeHotkey,
    /// The state machine was explicitly reset.
    Reset,
}

/// Why a crossing was accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossingReason {
    /// The pointer stayed in the edge activation zone for the dwell duration.
    Dwell,
    /// The pointer crossed with sufficient outward velocity after entering the
    /// activation zone.
    Velocity,
}

/// A completed pointer crossing between displays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossingEvent {
    /// Source display.
    pub from_display: DisplayId,
    /// Destination display.
    pub to_display: DisplayId,
    /// Source edge.
    pub from_edge: Edge,
    /// Destination edge.
    pub to_edge: Edge,
    /// Pointer location in destination topology coordinates.
    pub position: Point,
    /// Sample sequence which completed the crossing.
    pub sequence: u64,
    /// Sample timestamp which completed the crossing.
    pub timestamp: Timestamp,
    /// Activation reason.
    pub reason: CrossingReason,
}

/// Result of feeding an observation or control event to the state machine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CrossingAction {
    /// No state transition occurred.
    None,
    /// A peer became the active display.
    Crossed {
        /// Completed crossing details.
        event: CrossingEvent,
    },
    /// The local display became active.
    ReturnedToLocal {
        /// Why local control was restored.
        reason: ReturnReason,
        /// Timestamp associated with the transition, if known.
        timestamp: Timestamp,
    },
}

impl CrossingAction {
    /// Returns the crossing event, if this action crossed to a peer.
    #[must_use]
    pub fn crossing(&self) -> Option<&CrossingEvent> {
        match self {
            Self::Crossed { event } => Some(event),
            _ => None,
        }
    }

    /// Returns whether local control was restored.
    #[must_use]
    pub const fn returned_to_local(&self) -> bool {
        matches!(self, Self::ReturnedToLocal { .. })
    }
}

/// Escape key configuration. By default a pressed Escape key, with any
/// currently active modifiers, returns control to the local display.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EscapeHotkey {
    /// Key to watch.
    pub key: KeyCode,
    /// Modifiers which must be present.
    pub modifiers: Modifiers,
}

impl Default for EscapeHotkey {
    fn default() -> Self {
        Self {
            key: KeyCode::ESCAPE,
            modifiers: Modifiers::NONE,
        }
    }
}

impl EscapeHotkey {
    /// Creates an escape hotkey descriptor.
    #[must_use]
    pub const fn new(key: KeyCode, modifiers: Modifiers) -> Self {
        Self { key, modifiers }
    }

    /// Returns whether a key event matches this hotkey.
    #[must_use]
    pub fn matches(self, key: KeyCode, pressed: bool, modifiers: Modifiers) -> bool {
        pressed && key == self.key && modifiers.contains(self.modifiers)
    }
}

/// 穿越何时生效。
///
/// 三种模式对应生产的三个切换档，区别只在「候选要满足什么才放行」。几何判定与落点
/// 计算完全共用，因此模式之间不会产生落点差异。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivationMode {
    /// 触边即刻穿越。最激进，指针在边缘抖动时会反复穿越。
    Instant,
    /// 在边缘持续停留指定毫秒后穿越。
    Dwell(Timestamp),
    /// 离开同一条边之后，在指定毫秒内再次触边才穿越。
    DoubleTap(Timestamp),
}

impl Default for ActivationMode {
    fn default() -> Self {
        Self::Dwell(150)
    }
}

/// Thresholds for edge activation and re-arming.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossingConfig {
    /// 本机屏上距边缘多少像素以内算触边。远端不看这个值：远端指针必须真的越界。
    pub edge_threshold: f64,
    /// 穿越何时生效。
    pub activation: ActivationMode,
    /// Minimum outward velocity in logical pixels/second. Zero makes dwell
    /// sufficient, which is useful for a stationary pointer at an edge.
    pub min_outward_velocity: f64,
    /// Escape hotkey.
    pub escape_hotkey: EscapeHotkey,
}

impl Default for CrossingConfig {
    fn default() -> Self {
        Self {
            edge_threshold: 1.0,
            activation: ActivationMode::default(),
            min_outward_velocity: 0.0,
            escape_hotkey: EscapeHotkey::default(),
        }
    }
}

impl CrossingConfig {
    /// 构造停留指定毫秒后生效的配置。
    #[must_use]
    pub const fn with_dwell_ms(dwell_ms: Timestamp) -> Self {
        Self {
            edge_threshold: 1.0,
            activation: ActivationMode::Dwell(dwell_ms),
            min_outward_velocity: 0.0,
            escape_hotkey: EscapeHotkey {
                key: KeyCode::ESCAPE,
                modifiers: Modifiers::NONE,
            },
        }
    }

    /// Sets the edge activation distance.
    #[must_use]
    pub const fn with_edge_threshold(mut self, edge_threshold: f64) -> Self {
        self.edge_threshold = edge_threshold;
        self
    }

    /// Sets the minimum outward velocity.
    #[must_use]
    pub const fn with_min_velocity(mut self, min_outward_velocity: f64) -> Self {
        self.min_outward_velocity = min_outward_velocity;
        self
    }

    /// 设置穿越的生效方式。
    #[must_use]
    pub const fn with_activation(mut self, activation: ActivationMode) -> Self {
        self.activation = activation;
        self
    }

    /// Sets the hotkey.
    #[must_use]
    pub const fn with_escape_hotkey(mut self, escape_hotkey: EscapeHotkey) -> Self {
        self.escape_hotkey = escape_hotkey;
        self
    }
}

#[derive(Clone, Debug)]
struct EdgeCandidate {
    source: DisplayId,
    edge: Edge,
    mapping: EdgeMapping,
    started_at: Timestamp,
    last_sample: PointerSample,
    max_outward_velocity: f64,
    /// 候选期间是否离开过同一条边。双击模式靠它区分「首次触边」与「再触边」。
    left: bool,
}

/// 本机屏边缘的一点触发区宽度。
///
/// 操作系统把本机指针钳制在屏内，移交发生在最后一个像素而不是屏外一像素，因此
/// 本机源测量越界量时要按这个宽度外扩源边，落进本机屏时也要相应内缩。
const LOCAL_EDGE_ZONE: f64 = 1.0;

/// Pure state machine for edge crossing, escape recovery, and disconnect
/// recovery. It does not read clocks, sockets, or operating-system input APIs.
#[derive(Clone, Debug)]
pub struct CrossingStateMachine {
    topology: Topology,
    config: CrossingConfig,
    local_displays: BTreeSet<DisplayId>,
    local_display: DisplayId,
    active_display: DisplayId,
    connection: ConnectionState,
    candidate: Option<EdgeCandidate>,
    previous_sample: Option<PointerSample>,
    last_sequence: Option<u64>,
}

impl CrossingStateMachine {
    /// Creates a state machine. The topology's local display is the fallback
    /// display for disconnect and escape recovery.
    #[must_use]
    pub fn new(topology: Topology, config: CrossingConfig) -> Self {
        let local_display = topology.local_display.clone();
        Self {
            topology,
            config,
            local_displays: BTreeSet::from([local_display.clone()]),
            local_display: local_display.clone(),
            active_display: local_display,
            connection: ConnectionState::Connected,
            candidate: None,
            previous_sample: None,
            last_sequence: None,
        }
    }

    /// Creates a state machine with default thresholds.
    #[must_use]
    pub fn with_defaults(topology: Topology) -> Self {
        Self::new(topology, CrossingConfig::default())
    }

    /// Returns a reference to the topology.
    #[must_use]
    pub const fn topology(&self) -> &Topology {
        &self.topology
    }

    /// Returns the currently active display.
    #[must_use]
    pub const fn active_display(&self) -> &DisplayId {
        &self.active_display
    }

    /// Returns the connection state.
    #[must_use]
    pub const fn connection_state(&self) -> ConnectionState {
        self.connection
    }

    /// Updates the topology without changing local/active display state.
    pub fn set_topology(&mut self, topology: Topology) {
        self.topology = topology;
        self.local_display = self.topology.local_display.clone();
        if self.topology.placement(&self.active_display).is_none() {
            self.active_display = self.local_display.clone();
        }
        self.candidate = None;
    }

    /// Sets a new crossing configuration and clears an in-flight candidate.
    pub fn set_config(&mut self, config: CrossingConfig) {
        self.config = config;
        self.candidate = None;
    }

    /// Returns the current crossing configuration.
    #[must_use]
    pub const fn config(&self) -> CrossingConfig {
        self.config
    }

    /// 声明属于本机的全部屏幕。
    ///
    /// 本机可能有多块屏，而拓扑只把其中一块标为本地屏（控制器），因此这份集合必须
    /// 由调用方完整给出；构造时的本地屏始终保留在集合内。变更只清除在途候选：几何
    /// 没有变化，已经建立的穿越与锁定边仍然有效。
    pub fn set_local_displays(&mut self, displays: impl IntoIterator<Item = DisplayId>) {
        let mut local = BTreeSet::from([self.local_display.clone()]);
        local.extend(displays);
        self.local_displays = local;
        self.candidate = None;
    }

    /// 该屏需要的一点触发区补偿：本机屏为 [`LOCAL_EDGE_ZONE`]，远端为 `0.0`。
    fn local_zone(&self, display: &DisplayId) -> f64 {
        if self.local_displays.contains(display) {
            LOCAL_EDGE_ZONE
        } else {
            0.0
        }
    }

    /// Changes connection state. A disconnect while controlling a peer returns
    /// local control immediately and exactly once.
    pub fn set_connection(
        &mut self,
        state: ConnectionState,
        timestamp: Timestamp,
    ) -> CrossingAction {
        self.connection = state;
        self.candidate = None;
        if state == ConnectionState::Disconnected {
            self.return_local(ReturnReason::Disconnected, timestamp)
        } else {
            CrossingAction::None
        }
    }

    /// Convenience method for disconnect recovery.
    pub fn disconnect(&mut self, timestamp: Timestamp) -> CrossingAction {
        self.set_connection(ConnectionState::Disconnected, timestamp)
    }

    /// Convenience method for reconnecting. Reconnection itself has no input
    /// side effect; the next edge crossing can be observed normally.
    pub fn reconnect(&mut self) {
        self.connection = ConnectionState::Connected;
        self.candidate = None;
        self.previous_sample = None;
        self.last_sequence = None;
    }

    /// Explicitly returns control to local and clears all transient state.
    pub fn reset(&mut self, timestamp: Timestamp) -> CrossingAction {
        self.connection = ConnectionState::Connected;
        self.candidate = None;
        self.previous_sample = None;
        self.last_sequence = None;
        self.return_local(ReturnReason::Reset, timestamp)
    }

    /// Feeds one pointer sample to the state machine.
    pub fn process_sample(&mut self, mut sample: PointerSample) -> CrossingAction {
        if let Some(previous_sequence) = self.last_sequence
            && sample.sequence != 0
            && previous_sequence != 0
            && sample.sequence <= previous_sequence
        {
            return CrossingAction::None;
        }
        if sample.sequence != 0 {
            self.last_sequence = Some(sample.sequence);
        }

        if self.connection == ConnectionState::Disconnected {
            self.previous_sample = Some(sample);
            return CrossingAction::None;
        }

        if sample.velocity.magnitude() <= f64::EPSILON
            && let Some(previous) = self.previous_sample.as_ref()
        {
            sample.velocity = sample.velocity_from(previous);
        }
        self.previous_sample = Some(sample.clone());

        // The platform adapter may report display changes directly after a
        // crossing. Accept that acknowledgement.
        if sample.display_id != self.active_display {
            if self.topology.placement(&sample.display_id).is_some() {
                self.active_display = sample.display_id.clone();
            }
            self.candidate = None;
        }

        let Some((edge, mapping)) = self.find_unique_edge(&sample) else {
            self.retreat_candidate(&sample);
            return CrossingAction::None;
        };
        if mapping.from_display != self.active_display && mapping.from_display != sample.display_id
        {
            self.candidate = None;
            return CrossingAction::None;
        }

        let outward_velocity =
            sample.velocity.x * edge.outward().x + sample.velocity.y * edge.outward().y;
        let candidate_matches = self.candidate.as_ref().is_some_and(|candidate| {
            candidate.source == mapping.from_display
                && candidate.edge == edge
                && !self.candidate_expired(candidate, sample.timestamp)
        });
        if !candidate_matches {
            let timestamp = sample.timestamp;
            let sequence = sample.sequence;
            self.candidate = Some(EdgeCandidate {
                source: mapping.from_display.clone(),
                edge,
                mapping,
                started_at: sample.timestamp,
                last_sample: sample,
                max_outward_velocity: outward_velocity.max(0.0),
                left: false,
            });
            // 瞬时模式（以及停留窗口为 0 的配置）在第一个触边样本就穿越，
            // 没有等待期，不该要求第二个样本才放行。
            return self.complete_candidate(timestamp, sequence);
        }

        let candidate = self.candidate.as_mut().expect("candidate was checked");
        candidate.last_sample = sample.clone();
        candidate.max_outward_velocity = candidate.max_outward_velocity.max(outward_velocity);
        self.complete_candidate(sample.timestamp, sample.sequence)
    }

    /// 在没有新样本时推进当前候选。
    ///
    /// 生产里指针静止停在边缘时 dwell 由定时器完成，不会再有 motion 样本进来；
    /// 没有这条路径，静止停留就永远无法触发穿越。
    pub fn tick(&mut self, timestamp: Timestamp) -> CrossingAction {
        if self.connection == ConnectionState::Disconnected {
            return CrossingAction::None;
        }
        let sequence = self
            .candidate
            .as_ref()
            .map_or(0, |candidate| candidate.last_sample.sequence);
        self.complete_candidate(timestamp, sequence)
    }

    /// 用给定时刻评估当前候选是否满足穿越条件，满足则完成穿越。
    ///
    /// `now` 与 `sequence` 由调用方提供：样本驱动时用样本自身的时刻与序号，
    /// 定时器驱动时用定时器时刻与候选最后样本的序号。
    fn complete_candidate(&mut self, now: Timestamp, sequence: u64) -> CrossingAction {
        let Some(candidate) = self.candidate.as_ref() else {
            return CrossingAction::None;
        };
        let elapsed = now.saturating_sub(candidate.started_at);
        let ready = match self.config.activation {
            ActivationMode::Instant => true,
            ActivationMode::Dwell(window) => elapsed >= window,
            ActivationMode::DoubleTap(window) => candidate.left && elapsed <= window,
        };
        if !ready || candidate.max_outward_velocity < self.config.min_outward_velocity {
            return CrossingAction::None;
        }

        let candidate = self.candidate.take().expect("candidate exists");
        if self
            .topology
            .placement(&candidate.mapping.to_display)
            .is_none()
        {
            return CrossingAction::None;
        }
        let source_rect = self
            .topology
            .placement(&candidate.mapping.from_display)
            .map(|placement| placement.rect)
            .unwrap_or_default();
        // 源外扩与目标内缩都只对本机屏生效：远端指针报告的是真实越界量，越界多少
        // 就落进目标屏多少。
        let source_zone = self.local_zone(&candidate.mapping.from_display);
        let target_inset = self.local_zone(&candidate.mapping.to_display);
        let position = candidate.mapping.map_landing(
            candidate.last_sample.position,
            source_rect,
            source_zone,
            target_inset,
        );
        self.active_display = candidate.mapping.to_display.clone();
        let reason = if candidate.max_outward_velocity > 0.0 {
            CrossingReason::Velocity
        } else {
            CrossingReason::Dwell
        };
        CrossingAction::Crossed {
            event: CrossingEvent {
                from_display: candidate.mapping.from_display,
                to_display: candidate.mapping.to_display,
                from_edge: candidate.mapping.from_edge,
                to_edge: candidate.mapping.to_edge,
                position,
                sequence,
                timestamp: now,
                reason,
            },
        }
    }

    /// Feeds an input event, handling motion and the configured escape hotkey.
    pub fn process_event(&mut self, event: &InputEvent) -> CrossingAction {
        match event {
            InputEvent::Motion {
                display_id,
                position,
                velocity,
                sequence,
                timestamp,
            } => self.process_sample(PointerSample {
                display_id: display_id.clone(),
                position: *position,
                timestamp: *timestamp,
                sequence: *sequence,
                velocity: *velocity,
            }),
            InputEvent::Key {
                key,
                pressed,
                modifiers,
                timestamp,
                ..
            } if self
                .config
                .escape_hotkey
                .matches(*key, *pressed, *modifiers) =>
            {
                self.return_local(ReturnReason::EscapeHotkey, *timestamp)
            }
            InputEvent::Key { .. }
            | InputEvent::Button { .. }
            | InputEvent::Scroll { .. }
            | InputEvent::Modifiers { .. } => CrossingAction::None,
        }
    }

    /// Returns a copy of the active display's placement.
    #[must_use]
    pub fn active_placement(&self) -> Option<&crate::DevicePlacement> {
        self.topology.placement(&self.active_display)
    }

    /// 找到样本所在的唯一可穿越边。
    ///
    /// 本机与远端的触发条件刻意不同。本机指针被系统钳制在屏内，只在最外侧
    /// [`CrossingConfig::edge_threshold`] 个像素内移交；远端指针由增量积分得到，
    /// 必须**真的越界**才算触边，否则指针一靠近边缘就会穿越。这就是生产的
    /// primary/secondary 规则。
    fn find_unique_edge(&self, sample: &PointerSample) -> Option<(Edge, EdgeMapping)> {
        let placement = self.topology.placement(&sample.display_id)?;
        let local = self.local_displays.contains(&sample.display_id);
        let threshold = self.config.edge_threshold.max(0.0);
        let mut candidates = [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom]
            .into_iter()
            .filter_map(|edge| {
                let distance = placement
                    .rect
                    .signed_distance_to_edge(sample.position, edge);
                let on_edge = if local {
                    (0.0..=threshold).contains(&distance)
                } else {
                    distance < 0.0
                };
                if !on_edge {
                    return None;
                }
                self.topology
                    .mapping_for(&sample.display_id, edge)
                    .map(|mapping| (edge, mapping, distance.abs()))
            })
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return None;
        }
        candidates.sort_by(|a, b| a.2.total_cmp(&b.2));
        if candidates.len() > 1 && (candidates[0].2 - candidates[1].2).abs() <= 0.001 {
            // At a corner there is no deterministic route. Requiring the
            // pointer to move away from one side avoids accidental switching.
            return None;
        }
        let (edge, mapping, _) = candidates.remove(0);
        Some((edge, mapping))
    }

    /// 双击窗口过期后候选必须重新计时。
    ///
    /// 否则一次超时的触边会让这条边永久失效：候选还在、`left` 也还留着，但窗口
    /// 已经关不上，后续再触边也永远放行不了。
    fn candidate_expired(&self, candidate: &EdgeCandidate, now: Timestamp) -> bool {
        matches!(
            self.config.activation,
            ActivationMode::DoubleTap(window)
                if now.saturating_sub(candidate.started_at) > window
        )
    }

    /// 样本离开边缘区时处理在途候选。
    ///
    /// 停留模式在窗口内保持候选：远端落点可能停在边缘内侧，指针沿该边滑动时不该
    /// 取消已经开始的停留。双击模式记录「已离开同一条边」——这正是第二次触边的
    /// 判据。瞬时模式没有等待，直接丢弃候选。
    fn retreat_candidate(&mut self, sample: &PointerSample) {
        let Some(elapsed) = self
            .candidate
            .as_ref()
            .map(|candidate| sample.timestamp.saturating_sub(candidate.started_at))
        else {
            return;
        };
        match self.config.activation {
            ActivationMode::Dwell(window) if elapsed <= window => {}
            ActivationMode::DoubleTap(window) if elapsed <= window => {
                if let Some(candidate) = self.candidate.as_mut() {
                    candidate.left = true;
                }
            }
            _ => self.candidate = None,
        }
    }

    fn return_local(&mut self, reason: ReturnReason, timestamp: Timestamp) -> CrossingAction {
        let was_remote = self.active_display != self.local_display;
        self.active_display = self.local_display.clone();
        self.candidate = None;
        if was_remote {
            CrossingAction::ReturnedToLocal { reason, timestamp }
        } else {
            CrossingAction::None
        }
    }
}

// A private alias prevents accidental reliance on a similarly named type in a
// future protocol crate.
#[allow(dead_code)]
type _Velocity = Vector2;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DevicePlacement, Rect};

    fn machine(config: CrossingConfig) -> CrossingStateMachine {
        let topology = Topology::with_placements(
            "main",
            [
                DevicePlacement::controller("main", Rect::new(0.0, 0.0, 100.0, 100.0)),
                DevicePlacement::controlled("right", Rect::new(100.0, 0.0, 100.0, 100.0)),
                DevicePlacement::controlled("below", Rect::new(0.0, 100.0, 100.0, 100.0)),
            ],
        )
        .expect("valid topology");
        CrossingStateMachine::new(topology, config)
    }

    fn sample(x: f64, y: f64, timestamp: u64, sequence: u64) -> PointerSample {
        PointerSample::from_xy("main", x, y, timestamp, sequence)
    }

    #[test]
    fn crossing_after_dwell_routes_right() {
        let mut state = machine(CrossingConfig::with_dwell_ms(100));
        assert_eq!(
            state.process_sample(sample(100.0, 50.0, 0, 1)),
            CrossingAction::None
        );
        let action = state.process_sample(sample(100.0, 50.0, 100, 2));
        let event = action.crossing().expect("crossing");
        assert_eq!(event.to_display, DisplayId::from("right"));
        assert_eq!(event.to_edge, Edge::Left);
    }

    #[test]
    fn crossing_after_dwell_routes_down() {
        let mut state = machine(CrossingConfig::with_dwell_ms(10));
        assert_eq!(
            state.process_sample(sample(50.0, 99.0, 0, 1)),
            CrossingAction::None
        );
        let action = state.process_sample(sample(50.0, 100.0, 10, 2));
        assert_eq!(
            action.crossing().expect("crossing").to_display,
            DisplayId::from("below")
        );
    }

    #[test]
    fn corner_is_not_routed_when_both_edges_are_equally_close() {
        let mut state = machine(CrossingConfig::with_dwell_ms(0));
        assert_eq!(
            state.process_sample(sample(100.0, 100.0, 0, 1)),
            CrossingAction::None
        );
        assert_eq!(
            state.process_sample(sample(100.0, 100.0, 1, 2)),
            CrossingAction::None
        );
    }

    /// 生产允许真正的反向位移立即穿回，不等任何冷却或撤离保护。对齐
    /// `instant_crossing_and_immediate_return_are_allowed_without_inset_jump`：本机
    /// 穿到远端后，指针向回越界一个像素就穿回本机。
    #[test]
    fn instant_mode_allows_an_immediate_reverse_crossing() {
        let mut state = machine(CrossingConfig {
            activation: ActivationMode::Instant,
            ..CrossingConfig::default()
        });
        assert_eq!(
            state
                .process_sample(sample(100.0, 50.0, 0, 1))
                .crossing()
                .expect("forward crossing")
                .to_display,
            DisplayId::from("right")
        );
        // 远端屏的矩形是 (100, 0, 100, 100)：拓扑 x=99 表示已越界一个像素。
        assert_eq!(
            state
                .process_sample(PointerSample::from_xy("right", 99.0, 50.0, 1, 2))
                .crossing()
                .expect("reverse crossing")
                .to_display,
            DisplayId::from("main")
        );
    }

    #[test]
    fn disconnect_and_escape_return_local() {
        let mut state = machine(CrossingConfig::with_dwell_ms(0));
        // 停留窗口为 0 等价于瞬时：第一个触边样本就穿越。
        assert!(
            state
                .process_sample(sample(100.0, 50.0, 0, 1))
                .crossing()
                .is_some()
        );
        assert_eq!(
            state.disconnect(10),
            CrossingAction::ReturnedToLocal {
                reason: ReturnReason::Disconnected,
                timestamp: 10,
            }
        );
        assert_eq!(state.disconnect(11), CrossingAction::None);
        state.reconnect();
        assert_eq!(
            state.process_event(&InputEvent::key(
                KeyCode::ESCAPE,
                true,
                Modifiers::NONE,
                20,
                2
            )),
            CrossingAction::None
        );
    }

    /// 锁定 `PointerSample::position` 的坐标约定：它是拓扑空间坐标，不是屏内坐标。
    ///
    /// "right" 的矩形是 (100, 0, 100, 100)，其左边缘在拓扑空间里位于 x=100。
    /// 远端屏要求指针真的越界，所以 x=100（恰好贴边）不触发，x=99（越界一个单位）
    /// 才触发并切回 "main"。若该字段被当成屏内坐标，这两个结论会完全相反。
    #[test]
    fn pointer_sample_position_is_topology_space_not_display_local() {
        // 拓扑解释：x=100 距 "right" 的左边缘 0 个单位，尚未越界。
        let mut state = machine(CrossingConfig::with_dwell_ms(0));
        assert_eq!(
            state.process_sample(PointerSample::from_xy("right", 100.0, 50.0, 0, 1)),
            CrossingAction::None
        );
        assert_eq!(
            state.process_sample(PointerSample::from_xy("right", 100.0, 50.0, 1, 2)),
            CrossingAction::None
        );

        // 拓扑解释：x=99 已越出 "right" 的左边缘一个单位，算真越界。
        let mut state = machine(CrossingConfig::with_dwell_ms(0));
        assert_eq!(
            state
                .process_sample(PointerSample::from_xy("right", 99.0, 50.0, 0, 1))
                .crossing()
                .expect("return crossing")
                .to_display,
            DisplayId::from("main")
        );
    }

    /// 静止停在边缘时不会再有新样本，dwell 必须能由定时器完成。
    /// 对齐生产的 `stationary_dwell_fires_at_250`。
    #[test]
    fn stationary_dwell_completes_from_tick_without_new_samples() {
        let mut state = machine(CrossingConfig::with_dwell_ms(250));
        assert_eq!(
            state.process_sample(sample(100.0, 50.0, 0, 1)),
            CrossingAction::None
        );
        assert_eq!(state.tick(249), CrossingAction::None);

        let action = state.tick(250);
        let event = action.crossing().expect("timer completes the dwell");
        assert_eq!(event.to_display, DisplayId::from("right"));
        assert_eq!(event.to_edge, Edge::Left);
        // 完成时刻取定时器时刻，序号取候选最后样本的序号。
        assert_eq!(event.timestamp, 250);
        assert_eq!(event.sequence, 1);
        assert_eq!(state.active_display(), &DisplayId::from("right"));
    }

    /// 定时器只能完成已存在的候选，不能自己发起穿越：否则指针停在本机边缘
    /// 就会被定时器擅自推出本机。
    #[test]
    fn tick_never_starts_a_crossing_on_its_own() {
        let mut state = machine(CrossingConfig::with_dwell_ms(0));
        assert_eq!(state.tick(0), CrossingAction::None);
        assert_eq!(state.tick(10_000), CrossingAction::None);
        assert_eq!(state.active_display(), &DisplayId::from("main"));
    }

    /// 断线期间定时器必须保持静默，归还只由 `disconnect`/`reset` 触发一次。
    #[test]
    fn tick_is_inert_while_disconnected() {
        let mut state = machine(CrossingConfig::with_dwell_ms(250));
        assert_eq!(
            state.process_sample(sample(100.0, 50.0, 0, 1)),
            CrossingAction::None
        );
        state.disconnect(1);
        assert_eq!(state.tick(10_000), CrossingAction::None);
        assert_eq!(state.connection_state(), ConnectionState::Disconnected);
    }

    /// dwell 未满时定时器不得提前放行。
    #[test]
    fn tick_respects_the_dwell_window() {
        let mut state = machine(CrossingConfig::with_dwell_ms(100));
        assert_eq!(
            state.process_sample(sample(100.0, 50.0, 0, 1)),
            CrossingAction::None
        );
        assert_eq!(state.tick(99), CrossingAction::None);
        assert!(state.tick(100).crossing().is_some());
    }

    /// 两块副屏串在本机主屏右侧：`second` 可被声明为本机副屏或远端屏。
    fn second_screen_machine() -> CrossingStateMachine {
        let topology = Topology::with_placements(
            "main",
            [
                DevicePlacement::controller("main", Rect::new(0.0, 0.0, 100.0, 100.0)),
                DevicePlacement::controlled("second", Rect::new(100.0, 0.0, 100.0, 100.0)),
                DevicePlacement::controlled("third", Rect::new(200.0, 0.0, 100.0, 100.0)),
            ],
        )
        .expect("valid topology");
        CrossingStateMachine::new(topology, CrossingConfig::with_dwell_ms(250))
    }

    /// 本机屏集合至少包含拓扑的本地屏，并接受调用方追加本机副屏。
    #[test]
    fn set_local_displays_keeps_the_local_screen_and_extends_with_the_rest() {
        let mut state = machine(CrossingConfig::default());
        assert!(state.local_zone(&DisplayId::from("main")) > 0.0);
        assert_eq!(state.local_zone(&DisplayId::from("right")), 0.0);
        state.set_local_displays([DisplayId::from("right")]);
        assert!(state.local_zone(&DisplayId::from("right")) > 0.0);
        // 本地屏不会被追加项挤掉。
        assert!(state.local_zone(&DisplayId::from("main")) > 0.0);
    }

    /// 本机屏集合同时决定源的触发规则和一点触发区补偿。
    ///
    /// 远端源必须真的越界才触边，落点保留越界量；本机源被系统钳制在屏内，贴边
    /// 即触边，落点再按触发区补偿一个像素。样本因此不能共用：本机指针永远不会
    /// 越界到屏外。
    #[test]
    fn local_displays_decide_the_one_point_zone_on_landing() {
        // 远端源：越界一个像素才触边，落点保留这一个像素。
        let mut remote_source = second_screen_machine();
        assert_eq!(
            remote_source.process_sample(PointerSample::from_xy("second", 201.0, 50.0, 0, 1)),
            CrossingAction::None
        );
        let action = remote_source.tick(250);
        let crossed = action.crossing().expect("dwell completes the crossing");
        assert_eq!(crossed.position, Point::new(201.0, 50.0));

        // 本机源：贴边（尚未越界）即触边，落点因触发区补偿再前进一个像素。
        let mut local_source = second_screen_machine();
        local_source.set_local_displays([DisplayId::from("second")]);
        assert_eq!(
            local_source.process_sample(PointerSample::from_xy("second", 200.0, 50.0, 0, 1)),
            CrossingAction::None
        );
        let action = local_source.tick(250);
        let crossed = action.crossing().expect("dwell completes the crossing");
        assert_eq!(crossed.position, Point::new(201.0, 50.0));
    }

    /// 双击模式必须先离开同一条边，再在窗口内回到该边才穿越。
    /// 对齐生产的 `double_tap_needs_leave_same_edge_within_500`。
    #[test]
    fn double_tap_requires_leaving_the_same_edge_within_the_window() {
        let mut state = machine(CrossingConfig {
            activation: ActivationMode::DoubleTap(500),
            ..CrossingConfig::default()
        });
        // 首次触边只建立候选。
        assert_eq!(
            state.process_sample(sample(100.0, 50.0, 0, 1)),
            CrossingAction::None
        );
        // 一直停在边缘不算「离开」，定时器也不能放行。
        assert_eq!(state.tick(250), CrossingAction::None);
        assert_eq!(
            state.process_sample(sample(100.0, 50.0, 300, 2)),
            CrossingAction::None
        );
        // 退回屏内才算离开。
        assert_eq!(
            state.process_sample(sample(80.0, 50.0, 310, 3)),
            CrossingAction::None
        );
        // 窗口内再次触边，穿越。
        assert_eq!(
            state
                .process_sample(sample(100.0, 50.0, 400, 4))
                .crossing()
                .expect("second touch crosses")
                .to_display,
            DisplayId::from("right")
        );
    }

    /// 双击窗口过期后候选必须重新计时，否则这条边会永久失效。
    /// 对齐生产的 `double_tap_expiry_and_topology_reset`。
    #[test]
    fn double_tap_expiry_restarts_the_window() {
        let mut state = machine(CrossingConfig {
            activation: ActivationMode::DoubleTap(500),
            ..CrossingConfig::default()
        });
        assert_eq!(
            state.process_sample(sample(100.0, 50.0, 0, 1)),
            CrossingAction::None
        );
        assert_eq!(
            state.process_sample(sample(80.0, 50.0, 100, 2)),
            CrossingAction::None
        );
        // 501ms 已超出窗口：这次触边只是重新开始计时，不穿越。
        assert_eq!(
            state.process_sample(sample(100.0, 50.0, 501, 3)),
            CrossingAction::None
        );
        // 重新计时后，离开再回来即可穿越。
        assert_eq!(
            state.process_sample(sample(80.0, 50.0, 550, 4)),
            CrossingAction::None
        );
        assert_eq!(
            state
                .process_sample(sample(100.0, 50.0, 600, 5))
                .crossing()
                .expect("re-armed double tap crosses")
                .to_display,
            DisplayId::from("right")
        );
    }
}
