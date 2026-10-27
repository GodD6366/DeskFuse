//! Pure receive ordering for the negotiated reliable-control / datagram-motion mode.
//!
//! The caller must first authenticate the peer, transport, session/token, display,
//! topology and live lease. This state machine does not renew leases. Its returned
//! events are one atomic ordered batch and must retain that order through native
//! injection, including the reliable anchor before a button/key transition.

use deskfuse_core::{
    HybridInputUpdate, MotionDatagram, MotionPosition, NetworkError, NetworkErrorCode, Result,
    SharedInputEvent,
};

/// At most this many reliable sequence values may separate a future sample's epoch.
pub const MAX_FUTURE_STATE_EPOCH_DELTA: u64 = 64;
/// Maximum time to hold a sample waiting for its reliable state barrier.
pub const FUTURE_MOTION_TTL_MS: u64 = 100;

#[derive(Clone, Debug)]
struct PendingMotion {
    state_epoch: u64,
    position: MotionPosition,
    received_at_ms: u64,
}

/// Bounded per-session position receiver with independent motion/control watermarks.
///
/// Construct a fresh instance for each authorized lease; `leave` stops motion
/// until a strictly newer reliable Enter reopens it. Only reliable Enter/Transition messages advance `state_epoch`.
#[derive(Clone, Debug, Default)]
pub struct MotionReceiver {
    state_epoch: u64,
    latest_motion_seq: u64,
    reliable_watermark: u64,
    pending: Option<PendingMotion>,
    closed: bool,
    active_position: Option<MotionPosition>,
}

impl MotionReceiver {
    /// Start a new per-lease receiver. Future samples may wait for its first Enter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Latest applied input-state barrier, or zero before the reliable Enter.
    pub fn state_epoch(&self) -> u64 {
        self.state_epoch
    }

    /// Highest applied absolute-position sample number, shared by both channels.
    pub fn latest_motion_seq(&self) -> u64 {
        self.latest_motion_seq
    }

    /// 相对包只抬高水位，不产生绝对坐标事件。更晚到达的旧绝对包因此不能把光标拉回去。
    pub fn note_motion_seq(&mut self, motion_seq: u64) {
        if motion_seq > self.latest_motion_seq {
            self.latest_motion_seq = motion_seq;
        }
    }

    /// 可靠 Enter 之后、Leave 之前才接受不可靠移动。
    pub fn motion_open(&self) -> bool {
        !self.closed && self.state_epoch != 0
    }

    /// Receive reliable input and return its complete, indivisible injection batch.
    ///
    /// Reliable messages must have strictly increasing nonzero sequence numbers.
    /// Gaps are valid (renew/leave/control messages have their own runtime checks).
    /// A Snapshot consumes a reliable sequence but never advances the state epoch.
    /// `now_ms` is the receiving machine's monotonic clock in milliseconds.
    pub fn receive_reliable(
        &mut self,
        sequence: u64,
        update: &HybridInputUpdate,
        now_ms: u64,
    ) -> Result<Vec<SharedInputEvent>> {
        update.validate()?;
        if sequence == 0
            || sequence <= self.reliable_watermark
            || (self.closed && !matches!(update, HybridInputUpdate::Enter { .. }))
        {
            return Err(invalid(
                "inactive or non-increasing hybrid control sequence",
            ));
        }
        match update {
            HybridInputUpdate::Transition { .. } if self.state_epoch == 0 => {
                return Err(invalid("hybrid transition before enter"));
            }
            HybridInputUpdate::Snapshot { state_epoch, .. } if *state_epoch > sequence => {
                return Err(invalid("snapshot epoch exceeds reliable sequence"));
            }
            _ => {}
        }
        self.reliable_watermark = sequence;
        self.expire_pending(now_ms);
        let mut events = Vec::new();
        match update {
            HybridInputUpdate::Enter { position }
            | HybridInputUpdate::Transition { position, .. } => {
                // Do not suppress a reliable boundary because a newer position was
                // observed: the exact anchor is part of click/drag/key semantics.
                events.push(as_event(position));
                self.latest_motion_seq = self.latest_motion_seq.max(position.motion_seq);
                if let HybridInputUpdate::Transition {
                    events: changes, ..
                } = update
                {
                    events.extend(changes.iter().cloned());
                }
                self.state_epoch = sequence;
                self.closed = false;
                self.active_position = Some(position.clone());
                self.flush_pending(position, &mut events);
            }
            HybridInputUpdate::Snapshot {
                state_epoch,
                position,
            } => {
                self.accept_position(*state_epoch, position, now_ms, &mut events);
            }
        }
        Ok(events)
    }

    /// Receive a best-effort sample. Invalid, duplicate or stale samples are ignored.
    /// Future-state motion waits in a single latest slot and never crosses a barrier.
    pub fn receive_motion(
        &mut self,
        motion: &MotionDatagram,
        now_ms: u64,
    ) -> Vec<SharedInputEvent> {
        self.expire_pending(now_ms);
        let mut events = Vec::new();
        if !self.closed && motion.validate().is_ok() {
            self.accept_position(motion.state_epoch, &motion.position, now_ms, &mut events);
        }
        events
    }

    /// Stop motion and clear pending samples, preserving sequence watermarks.
    /// Only a strictly newer reliable Enter can reopen this receiver; late samples
    /// and duplicate Enter messages cannot resume control.
    pub fn leave(&mut self) {
        self.pending = None;
        self.active_position = None;
        self.closed = true;
    }

    /// Alias for `leave`, used by disconnect, permission-revocation and emergency paths.
    pub fn clear(&mut self) {
        self.leave();
    }

    fn accept_position(
        &mut self,
        state_epoch: u64,
        position: &MotionPosition,
        now_ms: u64,
        events: &mut Vec<SharedInputEvent>,
    ) {
        if state_epoch < self.state_epoch || position.motion_seq <= self.latest_motion_seq {
            return;
        }
        if state_epoch == self.state_epoch {
            if !self.active_position.as_ref().is_some_and(|anchor| {
                anchor.target_display == position.target_display
                    && anchor.topology_generation == position.topology_generation
            }) {
                return;
            }
            self.latest_motion_seq = position.motion_seq;
            events.push(as_event(position));
        } else if state_epoch - self.state_epoch <= MAX_FUTURE_STATE_EPOCH_DELTA
            && self.pending.as_ref().is_none_or(|old| {
                position.motion_seq > old.position.motion_seq && state_epoch >= old.state_epoch
            })
        {
            self.pending = Some(PendingMotion {
                state_epoch,
                position: position.clone(),
                received_at_ms: now_ms,
            });
        }
    }

    fn expire_pending(&mut self, now_ms: u64) {
        if self.pending.as_ref().is_some_and(|pending| {
            now_ms.saturating_sub(pending.received_at_ms) >= FUTURE_MOTION_TTL_MS
        }) {
            self.pending = None;
        }
    }

    fn flush_pending(&mut self, anchor: &MotionPosition, events: &mut Vec<SharedInputEvent>) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        if pending.position.target_display != anchor.target_display
            || pending.position.topology_generation != anchor.topology_generation
        {
            return;
        }
        if pending.state_epoch == self.state_epoch
            && pending.position.motion_seq > self.latest_motion_seq
        {
            self.latest_motion_seq = pending.position.motion_seq;
            events.push(as_event(&pending.position));
        } else if pending.state_epoch > self.state_epoch
            && pending.position.motion_seq > self.latest_motion_seq
        {
            self.pending = Some(pending);
        }
    }
}

fn as_event(position: &MotionPosition) -> SharedInputEvent {
    SharedInputEvent::Motion {
        x: position.x,
        y: position.y,
    }
}

fn invalid(message: &'static str) -> NetworkError {
    NetworkError::new(NetworkErrorCode::InvalidPayload, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use deskfuse_core::{DisplayId, InputSessionId, InputToken};

    fn position(seq: u64) -> MotionPosition {
        MotionPosition {
            target_display: DisplayId::new("main"),
            topology_generation: 1,
            motion_seq: seq,
            x: seq as f64,
            y: 1.,
        }
    }
    fn packet(epoch: u64, seq: u64) -> MotionDatagram {
        MotionDatagram {
            session_id: InputSessionId::new(),
            token: InputToken::new(),
            state_epoch: epoch,
            position: position(seq),
        }
    }
    fn motion(seq: u64) -> SharedInputEvent {
        as_event(&position(seq))
    }
    fn button(pressed: bool) -> SharedInputEvent {
        SharedInputEvent::Button { button: 0, pressed }
    }
    fn entered() -> MotionReceiver {
        let mut receiver = MotionReceiver::new();
        assert_eq!(
            receiver
                .receive_reliable(
                    2,
                    &HybridInputUpdate::Enter {
                        position: position(1)
                    },
                    0
                )
                .unwrap(),
            vec![motion(1)]
        );
        receiver
    }
    fn transition(seq: u64, pressed: bool) -> HybridInputUpdate {
        HybridInputUpdate::Transition {
            position: position(seq),
            events: vec![button(pressed)],
        }
    }

    #[test]
    fn reordered_and_duplicate_motion_never_moves_backwards() {
        let mut receiver = entered();
        assert_eq!(receiver.receive_motion(&packet(2, 5), 1), vec![motion(5)]);
        for seq in [4, 3, 2, 5] {
            assert!(receiver.receive_motion(&packet(2, seq), 2).is_empty());
        }
        assert_eq!(receiver.latest_motion_seq(), 5);
        assert_eq!(receiver.state_epoch(), 2);
    }

    #[test]
    fn future_drag_waits_for_button_down_and_is_batched_after_it() {
        let mut receiver = entered();
        assert!(receiver.receive_motion(&packet(3, 4), 1).is_empty());
        assert_eq!(
            receiver
                .receive_reliable(3, &transition(2, true), 2)
                .unwrap(),
            vec![motion(2), button(true), motion(4)]
        );
        assert_eq!(receiver.state_epoch(), 3);
        assert!(receiver.receive_motion(&packet(2, 100), 3).is_empty());
    }

    #[test]
    fn lost_final_drag_sample_releases_at_reliable_anchor() {
        let mut receiver = entered();
        receiver
            .receive_reliable(3, &transition(2, true), 1)
            .unwrap();
        receiver.receive_motion(&packet(3, 3), 2);
        // Final drag packet (4) was lost; release still moves to 4 while held.
        assert_eq!(
            receiver
                .receive_reliable(4, &transition(4, false), 3)
                .unwrap(),
            vec![motion(4), button(false)]
        );
        assert!(receiver.receive_motion(&packet(3, 5), 4).is_empty());
    }

    #[test]
    fn snapshot_converges_after_last_packet_loss_without_advancing_epoch() {
        let mut receiver = entered();
        let snapshot = HybridInputUpdate::Snapshot {
            state_epoch: 2,
            position: position(5),
        };
        assert_eq!(
            receiver.receive_reliable(6, &snapshot, 32).unwrap(),
            vec![motion(5)]
        );
        assert_eq!(receiver.state_epoch(), 2);
        assert_eq!(receiver.latest_motion_seq(), 5);
    }

    #[test]
    fn late_snapshot_cannot_overwrite_a_newer_datagram() {
        let mut receiver = entered();
        receiver.receive_motion(&packet(2, 8), 1);
        let snapshot = HybridInputUpdate::Snapshot {
            state_epoch: 2,
            position: position(5),
        };
        assert!(
            receiver
                .receive_reliable(6, &snapshot, 32)
                .unwrap()
                .is_empty()
        );
        assert_eq!(receiver.latest_motion_seq(), 8);
        assert_eq!(
            receiver
                .receive_reliable(7, &transition(9, true), 33)
                .unwrap(),
            vec![motion(9), button(true)]
        );
    }

    #[test]
    fn only_latest_future_sample_survives_and_skipped_epochs_are_safe() {
        let mut receiver = entered();
        receiver.receive_motion(&packet(3, 3), 1);
        receiver.receive_motion(&packet(5, 5), 2);
        receiver.receive_motion(&packet(3, 4), 3);
        assert_eq!(receiver.pending.as_ref().unwrap().position.motion_seq, 5);
        assert_eq!(
            receiver
                .receive_reliable(3, &transition(2, true), 4)
                .unwrap(),
            vec![motion(2), button(true)]
        );
        assert_eq!(
            receiver
                .receive_reliable(5, &transition(4, false), 5)
                .unwrap(),
            vec![motion(4), button(false), motion(5)]
        );
        assert!(receiver.pending.is_none());
    }

    #[test]
    fn future_slot_has_epoch_and_ttl_limits() {
        let mut receiver = entered();
        receiver.receive_motion(&packet(67, 10), 1);
        assert!(receiver.pending.is_none());
        receiver.receive_motion(&packet(3, 5), 2);
        assert!(receiver.pending.is_some());
        assert_eq!(
            receiver
                .receive_reliable(3, &transition(2, true), 102)
                .unwrap(),
            vec![motion(2), button(true)]
        );
        assert!(receiver.pending.is_none());
    }

    #[test]
    fn future_can_arrive_before_enter_but_only_new_reliable_enter_reopens_leave() {
        let mut receiver = MotionReceiver::new();
        assert!(receiver.receive_motion(&packet(2, 3), 1).is_empty());
        let enter = HybridInputUpdate::Enter {
            position: position(1),
        };
        assert_eq!(
            receiver.receive_reliable(2, &enter, 2).unwrap(),
            vec![motion(1), motion(3)]
        );
        receiver.leave();
        assert!(receiver.receive_motion(&packet(2, 9), 3).is_empty());
        assert!(receiver.receive_reliable(2, &enter, 4).is_err());
        assert!(receiver.pending.is_none());
        assert_eq!(receiver.latest_motion_seq(), 3);
        assert_eq!(
            receiver.receive_reliable(4, &enter, 5).unwrap(),
            vec![motion(1)]
        );
        assert!(receiver.receive_motion(&packet(2, 10), 6).is_empty());
        assert_eq!(receiver.receive_motion(&packet(4, 10), 7), vec![motion(10)]);
    }

    #[test]
    fn reliable_sequence_gaps_are_valid_but_replays_and_pre_enter_transition_are_not() {
        let mut receiver = MotionReceiver::new();
        assert!(
            receiver
                .receive_reliable(1, &transition(1, true), 0)
                .is_err()
        );
        let mut receiver = entered();
        assert!(
            receiver
                .receive_reliable(2, &transition(2, true), 1)
                .is_err()
        );
        assert!(
            receiver
                .receive_reliable(9, &transition(2, true), 2)
                .is_ok()
        );
        assert_eq!(receiver.state_epoch(), 9);
        assert!(
            receiver
                .receive_reliable(8, &transition(3, false), 3)
                .is_err()
        );
    }

    #[test]
    fn mixed_reliable_events_keep_every_boundary_in_original_order() {
        let mut receiver = entered();
        receiver.receive_motion(&packet(3, 10), 1);
        let changes = vec![
            button(true),
            SharedInputEvent::Key {
                code: 1,
                pressed: true,
                modifiers: 0,
                repeat: false,
            },
            SharedInputEvent::Modifiers { modifiers: 4 },
            SharedInputEvent::Key {
                code: 1,
                pressed: false,
                modifiers: 4,
                repeat: false,
            },
            button(false),
            SharedInputEvent::scroll(1., 2.),
        ];
        let update = HybridInputUpdate::Transition {
            position: position(2),
            events: changes.clone(),
        };
        let mut expected = vec![motion(2)];
        expected.extend(changes);
        expected.push(motion(10));
        assert_eq!(receiver.receive_reliable(3, &update, 2).unwrap(), expected);
    }

    #[test]
    fn stale_epoch_snapshot_never_crosses_button_state_barrier() {
        let mut receiver = entered();
        receiver
            .receive_reliable(3, &transition(2, true), 1)
            .unwrap();
        let old = HybridInputUpdate::Snapshot {
            state_epoch: 2,
            position: position(100),
        };
        assert!(receiver.receive_reliable(4, &old, 2).unwrap().is_empty());
        assert_eq!(receiver.latest_motion_seq(), 2);
    }

    #[test]
    fn invalid_datagrams_do_not_mutate_watermarks() {
        let mut receiver = entered();
        let mut invalid = packet(2, 3);
        invalid.position.x = f64::NAN;
        assert!(receiver.receive_motion(&invalid, 1).is_empty());
        assert_eq!(receiver.latest_motion_seq(), 1);
    }

    #[test]
    fn pending_position_cannot_cross_target_or_topology_boundary() {
        for change_topology in [true, false] {
            let mut receiver = entered();
            let mut pending = packet(3, 5);
            if change_topology {
                pending.position.topology_generation = 2;
            } else {
                pending.position.target_display = DisplayId::new("other");
            }
            receiver.receive_motion(&pending, 1);
            assert_eq!(
                receiver
                    .receive_reliable(3, &transition(2, true), 2)
                    .unwrap(),
                vec![motion(2), button(true)]
            );
            assert!(receiver.pending.is_none());
            pending.state_epoch = 3;
            assert!(receiver.receive_motion(&pending, 3).is_empty());
            assert_eq!(receiver.latest_motion_seq(), 2);
        }
    }

    #[test]
    fn reliable_target_change_accepts_only_matching_pending_position() {
        let mut receiver = entered();
        let mut pending = packet(4, 5);
        pending.position.target_display = DisplayId::new("other");
        receiver.receive_motion(&pending, 1);
        let mut anchor = position(4);
        anchor.target_display = DisplayId::new("other");
        assert_eq!(
            receiver
                .receive_reliable(4, &HybridInputUpdate::Enter { position: anchor }, 2)
                .unwrap(),
            vec![motion(4), motion(5)]
        );
        assert!(receiver.receive_motion(&packet(4, 6), 3).is_empty());
    }

    #[test]
    fn future_snapshot_obeys_same_epoch_barrier_as_datagram() {
        let mut receiver = entered();
        let snapshot = HybridInputUpdate::Snapshot {
            state_epoch: 4,
            position: position(6),
        };
        assert!(
            receiver
                .receive_reliable(4, &snapshot, 1)
                .unwrap()
                .is_empty()
        );
        assert_eq!(receiver.state_epoch(), 2);
        // That state's control never arrives; a later barrier safely discards it.
        assert_eq!(
            receiver
                .receive_reliable(5, &transition(7, true), 2)
                .unwrap(),
            vec![motion(7), button(true)]
        );
        assert!(receiver.pending.is_none());
    }

    #[test]
    fn largest_transition_with_anchor_and_pending_fits_native_batch_bound() {
        let mut receiver = entered();
        receiver.receive_motion(&packet(3, 5), 1);
        let update = HybridInputUpdate::Transition {
            position: position(2),
            events: vec![SharedInputEvent::Modifiers { modifiers: 0 }; 254],
        };
        let events = receiver.receive_reliable(3, &update, 2).unwrap();
        assert_eq!(events.len(), 256);
        assert_eq!(events.first(), Some(&motion(2)));
        assert_eq!(events.last(), Some(&motion(5)));
    }

    #[test]
    fn unnumbered_transition_motion_is_rejected_without_mutating_receiver() {
        let mut receiver = entered();
        receiver.receive_motion(&packet(3, 5), 1);
        let invalid = HybridInputUpdate::Transition {
            position: position(2),
            events: vec![motion(4), button(true)],
        };
        assert!(receiver.receive_reliable(3, &invalid, 2).is_err());
        assert_eq!(receiver.state_epoch(), 2);
        assert_eq!(receiver.latest_motion_seq(), 1);
        // The failed control did not consume its sequence or cached future motion.
        assert_eq!(
            receiver
                .receive_reliable(3, &transition(2, true), 3)
                .unwrap(),
            vec![motion(2), button(true), motion(5)]
        );
    }
}
