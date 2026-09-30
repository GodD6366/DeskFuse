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

/// Thresholds for edge activation and re-arming.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossingConfig {
    /// Distance from an edge in which a sample can start a candidate crossing.
    pub edge_threshold: f64,
    /// Minimum time in the activation zone, in milliseconds.
    pub dwell_ms: Timestamp,
    /// Minimum outward velocity in logical pixels/second. Zero makes dwell
    /// sufficient, which is useful for a stationary pointer at an edge.
    pub min_outward_velocity: f64,
    /// Additional distance the pointer must move away before another crossing
    /// can be armed.
    pub hysteresis: f64,
    /// Distance to place the pointer inside the target after crossing.
    pub target_inset: f64,
    /// Escape hotkey.
    pub escape_hotkey: EscapeHotkey,
}

impl Default for CrossingConfig {
    fn default() -> Self {
        Self {
            edge_threshold: 2.0,
            dwell_ms: 150,
            min_outward_velocity: 0.0,
            hysteresis: 8.0,
            target_inset: 1.0,
            escape_hotkey: EscapeHotkey::default(),
        }
    }
}

impl CrossingConfig {
    /// Creates defaults with a custom dwell duration.
    #[must_use]
    pub const fn with_dwell_ms(dwell_ms: Timestamp) -> Self {
        Self {
            dwell_ms,
            edge_threshold: 2.0,
            min_outward_velocity: 0.0,
            hysteresis: 8.0,
            target_inset: 1.0,
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

    /// Sets the hysteresis distance.
    #[must_use]
    pub const fn with_hysteresis(mut self, hysteresis: f64) -> Self {
        self.hysteresis = hysteresis;
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
}

/// Pure state machine for edge crossing, escape recovery, and disconnect
/// recovery. It does not read clocks, sockets, or operating-system input APIs.
#[derive(Clone, Debug)]
pub struct CrossingStateMachine {
    topology: Topology,
    config: CrossingConfig,
    local_display: DisplayId,
    active_display: DisplayId,
    connection: ConnectionState,
    armed: bool,
    locked_edge: Option<(DisplayId, Edge)>,
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
            local_display: local_display.clone(),
            active_display: local_display,
            connection: ConnectionState::Connected,
            armed: true,
            locked_edge: None,
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

    /// Returns whether this state machine is currently armed for a crossing.
    #[must_use]
    pub const fn is_armed(&self) -> bool {
        self.armed
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
        self.locked_edge = None;
        self.armed = true;
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
        self.armed = true;
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

        if !self.armed {
            if self.can_rearm(&sample) {
                self.armed = true;
                self.locked_edge = None;
            } else {
                return CrossingAction::None;
            }
        }

        // The platform adapter may report display changes directly after a
        // crossing. Accept that acknowledgement while retaining hysteresis.
        if sample.display_id != self.active_display {
            if self.topology.placement(&sample.display_id).is_some() {
                self.active_display = sample.display_id.clone();
            }
            self.candidate = None;
        }

        let Some((edge, mapping)) = self.find_unique_edge(&sample) else {
            self.candidate = None;
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
            candidate.source == mapping.from_display && candidate.edge == edge
        });
        if !candidate_matches {
            self.candidate = Some(EdgeCandidate {
                source: mapping.from_display.clone(),
                edge,
                mapping,
                started_at: sample.timestamp,
                last_sample: sample,
                max_outward_velocity: outward_velocity.max(0.0),
            });
            return CrossingAction::None;
        }

        let candidate = self.candidate.as_mut().expect("candidate was checked");
        candidate.last_sample = sample.clone();
        candidate.max_outward_velocity = candidate.max_outward_velocity.max(outward_velocity);
        let dwell_elapsed = sample.timestamp.saturating_sub(candidate.started_at);
        let enough_dwell = dwell_elapsed >= self.config.dwell_ms;
        let enough_velocity = candidate.max_outward_velocity >= self.config.min_outward_velocity;
        if !enough_dwell || !enough_velocity {
            return CrossingAction::None;
        }

        let candidate = self.candidate.take().expect("candidate exists");
        let Some(target) = self.topology.placement(&candidate.mapping.to_display) else {
            return CrossingAction::None;
        };
        let source_rect = self
            .topology
            .placement(&candidate.mapping.from_display)
            .map(|placement| placement.rect)
            .unwrap_or_default();
        let position = candidate.mapping.map_point_with_rects(
            sample.position,
            source_rect,
            target.rect,
            self.config.target_inset,
        );
        self.active_display = candidate.mapping.to_display.clone();
        self.armed = false;
        self.locked_edge = Some((
            candidate.mapping.to_display.clone(),
            candidate.mapping.to_edge,
        ));
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
                sequence: sample.sequence,
                timestamp: sample.timestamp,
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

    fn find_unique_edge(&self, sample: &PointerSample) -> Option<(Edge, EdgeMapping)> {
        let placement = self.topology.placement(&sample.display_id)?;
        let threshold = self.config.edge_threshold.max(0.0);
        let mut candidates = [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom]
            .into_iter()
            .filter_map(|edge| {
                let distance = placement
                    .rect
                    .signed_distance_to_edge(sample.position, edge);
                if distance < -threshold || distance > threshold {
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

    fn can_rearm(&self, sample: &PointerSample) -> bool {
        let Some((display, edge)) = self.locked_edge.as_ref() else {
            return true;
        };
        if sample.display_id != *display {
            return true;
        }
        let Some(placement) = self.topology.placement(display) else {
            return true;
        };
        placement
            .rect
            .signed_distance_to_edge(sample.position, *edge)
            > self.config.edge_threshold.max(0.0) + self.config.hysteresis.max(0.0)
    }

    fn return_local(&mut self, reason: ReturnReason, timestamp: Timestamp) -> CrossingAction {
        let was_remote = self.active_display != self.local_display;
        self.active_display = self.local_display.clone();
        self.candidate = None;
        self.armed = false;
        self.locked_edge = None;
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
            state.process_sample(sample(98.0, 50.0, 0, 1)),
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

    #[test]
    fn hysteresis_prevents_immediate_return_and_rearms_after_moving_inside() {
        let mut state = machine(
            CrossingConfig::with_dwell_ms(0)
                .with_hysteresis(10.0)
                .with_edge_threshold(2.0),
        );
        assert_eq!(
            state.process_sample(sample(100.0, 50.0, 0, 1)),
            CrossingAction::None
        );
        assert!(
            state
                .process_sample(sample(100.0, 50.0, 1, 2))
                .crossing()
                .is_some()
        );
        // Still at target edge: no immediate reverse crossing.
        let mut remote = PointerSample::from_xy("right", 100.0, 50.0, 2, 3);
        assert_eq!(state.process_sample(remote.clone()), CrossingAction::None);
        remote.position.x = 120.0;
        remote.timestamp = 3;
        remote.sequence = 4;
        assert_eq!(state.process_sample(remote), CrossingAction::None);
        // Re-armed after moving inside, then dwell at the return edge.
        let back = PointerSample::from_xy("right", 100.0, 50.0, 4, 5);
        assert_eq!(state.process_sample(back), CrossingAction::None);
        assert_eq!(
            state
                .process_sample(PointerSample::from_xy("right", 100.0, 50.0, 5, 6))
                .crossing()
                .expect("return crossing")
                .to_display,
            DisplayId::from("main")
        );
    }

    #[test]
    fn disconnect_and_escape_return_local() {
        let mut state = machine(CrossingConfig::with_dwell_ms(0));
        assert_eq!(
            state.process_sample(sample(100.0, 50.0, 0, 1)),
            CrossingAction::None
        );
        assert!(
            state
                .process_sample(sample(100.0, 50.0, 1, 2))
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
}
