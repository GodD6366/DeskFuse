//! Single-consumer native-input delivery, separate from lossy UI broadcasts.
//!
//! Only adjacent position-only frames in the same input state may supersede
//! one another. Overflow revokes all input rather than losing an up-event.

use std::collections::VecDeque;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use deskfuse_core::SharedInputEvent;
use futures::stream::{self, BoxStream};
use tokio::sync::Notify;

use crate::RuntimeEvent;

const CAPACITY: usize = 256;

pub(crate) struct InputDelivery {
    state: Mutex<State>,
    ready: Notify,
    overflow: Arc<AtomicBool>,
    capacity: usize,
}

#[derive(Default)]
struct State {
    pending: VecDeque<RuntimeEvent>,
    subscribed: bool,
    closed: bool,
    invalidated: bool,
}

impl InputDelivery {
    pub(crate) fn new(overflow: Arc<AtomicBool>) -> Self {
        Self::with_capacity(overflow, CAPACITY)
    }

    fn with_capacity(overflow: Arc<AtomicBool>, capacity: usize) -> Self {
        Self {
            state: Mutex::new(State::default()),
            ready: Notify::new(),
            overflow,
            capacity: capacity.max(1),
        }
    }

    /// Return false when input was rejected and the actor must revoke leases.
    /// The actor observes `overflow` before accepting subsequent input work.
    pub(crate) fn emit(&self, mut event: RuntimeEvent) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            self.overflow.store(true, Ordering::Release);
            return false;
        }
        // The input actor clears overflow immediately before deactivating all
        // sessions. Its serial emission ensures those releases precede any new
        // session or input after recovery.
        if state.invalidated && !self.overflow.load(Ordering::Acquire) {
            state.invalidated = false;
        }
        if state.invalidated
            && matches!(
                event,
                RuntimeEvent::SharedInputReceived { .. } | RuntimeEvent::InputControlGranted { .. }
            )
        {
            return false;
        }
        if let RuntimeEvent::InputControlReleased { session_id, .. } = &event {
            state.pending.retain(|pending| {
                !matches!(pending,
                    RuntimeEvent::SharedInputReceived { session_id: queued, .. }
                    | RuntimeEvent::InputControlGranted { session_id: queued, .. }
                    | RuntimeEvent::InputControlReleased { session_id: queued, .. }
                    if queued == session_id)
            });
        }
        if is_motion(&event) {
            // A batch of absolute positions also has only one useful endpoint.
            if let RuntimeEvent::SharedInputReceived { events, .. } = &mut event {
                let last = events.pop().expect("nonempty motion batch");
                events.clear();
                events.push(last);
            }
            if let Some(last) = state.pending.back_mut()
                && can_coalesce(last, &event)
            {
                *last = event;
                drop(state);
                self.ready.notify_one();
                return true;
            }
        }
        if is_relative(&event) {
            let combined = state.pending.back().and_then(|last| {
                if !can_coalesce_relative(last, &event) {
                    return None;
                }
                let RuntimeEvent::SharedInputReceived {
                    events: previous, ..
                } = last
                else {
                    return None;
                };
                let RuntimeEvent::SharedInputReceived {
                    events: incoming,
                    sequence,
                    ..
                } = &event
                else {
                    return None;
                };
                let (left_x, left_y) = relative_sum(previous)?;
                let (right_x, right_y) = relative_sum(incoming)?;
                let dx = left_x + right_x;
                let dy = left_y + right_y;
                if !deskfuse_core::relative_delta_ok(dx) || !deskfuse_core::relative_delta_ok(dy) {
                    return None;
                }
                Some((*sequence, dx, dy))
            });
            if let Some((sequence, dx, dy)) = combined
                && let Some(RuntimeEvent::SharedInputReceived {
                    events,
                    sequence: queued,
                    ..
                }) = state.pending.back_mut()
            {
                *queued = sequence;
                *events = vec![SharedInputEvent::RelativeMotion { dx, dy }];
                drop(state);
                self.ready.notify_one();
                return true;
            }
        }
        if state.pending.len() >= self.capacity {
            self.invalidate(&mut state, "native input delivery queue overflow");
            drop(state);
            self.ready.notify_one();
            return false;
        }
        state.pending.push_back(event);
        drop(state);
        self.ready.notify_one();
        true
    }

    fn invalidate(&self, state: &mut State, reason: &str) {
        state.pending.clear();
        state
            .pending
            .push_back(RuntimeEvent::InputDeliveryInvalidated {
                reason: reason.into(),
            });
        state.invalidated = true;
        self.overflow.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn try_recv(&self) -> Result<RuntimeEvent, ()> {
        self.state.lock().unwrap().pending.pop_front().ok_or(())
    }

    /// A second subscription is empty; native input must never be injected twice.
    pub(crate) fn stream(self: &Arc<Self>) -> BoxStream<'static, RuntimeEvent> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.subscribed {
            return Box::pin(stream::empty());
        }
        state.subscribed = true;
        drop(state);
        let consumer = Consumer(self.clone());
        Box::pin(stream::unfold(consumer, |consumer| async move {
            let delivery = consumer.0.clone();
            loop {
                // Register before inspecting the queue to avoid lost wakeups.
                let notified = delivery.ready.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                {
                    let mut state = delivery.state.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(event) = state.pending.pop_front() {
                        return Some((event, consumer));
                    }
                    if state.closed {
                        return None;
                    }
                }
                notified.await;
            }
        }))
    }

    /// Closing cancels pending injections and always exposes a release-all edge.
    pub(crate) fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.closed {
            self.invalidate(&mut state, "native input delivery stopped");
            state.closed = true;
        }
        drop(state);
        self.ready.notify_one();
    }
}

struct Consumer(Arc<InputDelivery>);

impl Drop for Consumer {
    fn drop(&mut self) {
        // Dropping the only consumer cannot leave a live remote-input session.
        self.0.close();
    }
}

fn is_motion(event: &RuntimeEvent) -> bool {
    matches!(event, RuntimeEvent::SharedInputReceived { events, .. }
        if !events.is_empty() && events.iter().all(|event| matches!(event, SharedInputEvent::Motion { .. })))
}

fn is_relative(event: &RuntimeEvent) -> bool {
    matches!(event, RuntimeEvent::SharedInputReceived { events, .. }
        if !events.is_empty()
            && events
                .iter()
                .all(|event| matches!(event, SharedInputEvent::RelativeMotion { .. })))
}

fn relative_sum(events: &[SharedInputEvent]) -> Option<(f64, f64)> {
    let mut dx = 0.0;
    let mut dy = 0.0;
    for event in events {
        let SharedInputEvent::RelativeMotion { dx: x, dy: y } = event else {
            return None;
        };
        dx += *x;
        dy += *y;
    }
    if deskfuse_core::relative_delta_ok(dx) && deskfuse_core::relative_delta_ok(dy) {
        Some((dx, dy))
    } else {
        None
    }
}

fn can_coalesce(previous: &RuntimeEvent, next: &RuntimeEvent) -> bool {
    match (previous, next) {
        (
            RuntimeEvent::SharedInputReceived {
                peer_id: a_peer,
                session_id: a_session,
                target_display: a_display,
                state_epoch: a_epoch,
                sequence: a_sequence,
                ..
            },
            RuntimeEvent::SharedInputReceived {
                peer_id: b_peer,
                session_id: b_session,
                target_display: b_display,
                state_epoch: b_epoch,
                sequence: b_sequence,
                ..
            },
        ) => {
            is_motion(previous)
                && a_peer == b_peer
                && a_session == b_session
                && a_display == b_display
                && a_epoch == b_epoch
                && b_sequence > a_sequence
        }
        _ => false,
    }
}

fn can_coalesce_relative(previous: &RuntimeEvent, next: &RuntimeEvent) -> bool {
    match (previous, next) {
        (
            RuntimeEvent::SharedInputReceived {
                peer_id: a_peer,
                session_id: a_session,
                target_display: a_display,
                state_epoch: a_epoch,
                sequence: a_sequence,
                ..
            },
            RuntimeEvent::SharedInputReceived {
                peer_id: b_peer,
                session_id: b_session,
                target_display: b_display,
                state_epoch: b_epoch,
                sequence: b_sequence,
                ..
            },
        ) => {
            is_relative(previous)
                && a_peer == b_peer
                && a_session == b_session
                && a_display == b_display
                && a_epoch == b_epoch
                && b_sequence > a_sequence
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deskfuse_core::{DeviceId, DisplayId, InputSessionId};
    use futures::StreamExt;

    fn motion(session_id: InputSessionId, sequence: u64, state_epoch: u64) -> RuntimeEvent {
        RuntimeEvent::SharedInputReceived {
            peer_id: DeviceId::from_bytes([7; 32]),
            session_id,
            target_display: DisplayId("main".into()),
            sequence,
            state_epoch,
            events: vec![SharedInputEvent::Motion {
                x: sequence as f64,
                y: 1.0,
            }],
        }
    }

    fn next_motion(previous: &RuntimeEvent, sequence: u64) -> RuntimeEvent {
        let mut next = previous.clone();
        if let RuntimeEvent::SharedInputReceived {
            sequence: seq,
            events,
            ..
        } = &mut next
        {
            *seq = sequence;
            *events = vec![SharedInputEvent::Motion {
                x: sequence as f64,
                y: 1.0,
            }];
        }
        next
    }

    fn delivery(capacity: usize) -> Arc<InputDelivery> {
        Arc::new(InputDelivery::with_capacity(
            Arc::new(AtomicBool::new(false)),
            capacity,
        ))
    }

    #[tokio::test]
    async fn adjacent_motion_retains_only_latest_position() {
        let delivery = delivery(2);
        let first = motion(InputSessionId::new(), 1, 1);
        let second = next_motion(&first, 2);
        assert!(delivery.emit(first));
        assert!(delivery.emit(second.clone()));
        assert_eq!(delivery.state.lock().unwrap().pending.len(), 1);
        assert_eq!(delivery.stream().next().await, Some(second));
    }

    #[tokio::test]
    async fn mixed_button_frame_preserves_order_as_barrier() {
        let delivery = delivery(4);
        let first = motion(InputSessionId::new(), 1, 1);
        let mut button = next_motion(&first, 2);
        if let RuntimeEvent::SharedInputReceived { events, .. } = &mut button {
            events.push(SharedInputEvent::Button {
                button: 0,
                pressed: true,
            });
        }
        let last = next_motion(&first, 3);
        for event in [&first, &button, &last] {
            assert!(delivery.emit(event.clone()));
        }
        let mut stream = delivery.stream();
        assert_eq!(stream.next().await, Some(first));
        assert_eq!(stream.next().await, Some(button));
        assert_eq!(stream.next().await, Some(last));
    }

    #[test]
    fn epoch_display_session_and_old_sequence_do_not_coalesce() {
        let first = motion(InputSessionId::new(), 1, 1);
        for change in 0..4 {
            let mut next = next_motion(&first, 2);
            if let RuntimeEvent::SharedInputReceived {
                session_id,
                target_display,
                state_epoch,
                sequence,
                ..
            } = &mut next
            {
                match change {
                    0 => *session_id = InputSessionId::new(),
                    1 => *target_display = DisplayId("other".into()),
                    2 => *state_epoch += 1,
                    _ => *sequence = 0,
                }
            }
            assert!(!can_coalesce(&first, &next));
        }
    }

    #[tokio::test]
    async fn overflow_invalidates_all_input_and_cannot_lose_release() {
        let delivery = delivery(1);
        let first = motion(InputSessionId::new(), 1, 1);
        assert!(delivery.emit(first.clone()));
        let release = RuntimeEvent::InputControlReleased {
            peer_id: DeviceId::from_bytes([7; 32]),
            session_id: InputSessionId::new(),
            reason: "released".into(),
        };
        assert!(!delivery.emit(release));
        assert!(delivery.overflow.load(Ordering::Acquire));
        assert!(!delivery.emit(next_motion(&first, 2)));
        assert_eq!(delivery.state.lock().unwrap().pending.len(), 1);
        assert!(matches!(
            delivery.stream().next().await,
            Some(RuntimeEvent::InputDeliveryInvalidated { .. })
        ));
    }

    #[tokio::test]
    async fn release_cancels_session_injection_but_preserves_other_session() {
        let delivery = delivery(4);
        let session = InputSessionId::new();
        let first = motion(session, 1, 1);
        let other = motion(InputSessionId::new(), 1, 1);
        let release = RuntimeEvent::InputControlReleased {
            peer_id: DeviceId::from_bytes([7; 32]),
            session_id: session,
            reason: "released".into(),
        };
        delivery.emit(first);
        delivery.emit(other.clone());
        delivery.emit(release.clone());
        let mut stream = delivery.stream();
        assert_eq!(stream.next().await, Some(other));
        assert_eq!(stream.next().await, Some(release));
    }

    #[tokio::test]
    async fn only_one_consumer_and_drop_closes_bounded_delivery() {
        let delivery = delivery(1);
        let first = delivery.stream();
        assert!(delivery.stream().next().await.is_none());
        drop(first);
        assert!(!delivery.emit(motion(InputSessionId::new(), 1, 1)));
        assert!(delivery.overflow.load(Ordering::Acquire));
        assert!(delivery.state.lock().unwrap().pending.len() <= 1);
    }

    #[test]
    fn keyboard_and_control_grant_are_motion_barriers() {
        let delivery = delivery(8);
        let session = InputSessionId::new();
        let first = motion(session, 1, 1);
        let mut key = next_motion(&first, 2);
        if let RuntimeEvent::SharedInputReceived { events, .. } = &mut key {
            *events = vec![SharedInputEvent::Key {
                code: 1,
                pressed: true,
                modifiers: 0,
                repeat: false,
            }];
        }
        delivery.emit(first.clone());
        delivery.emit(key);
        delivery.emit(next_motion(&first, 3));
        delivery.emit(RuntimeEvent::InputControlGranted {
            peer_id: DeviceId::from_bytes([7; 32]),
            session_id: session,
            lease_expires_at: 100,
        });
        delivery.emit(next_motion(&first, 4));
        assert_eq!(delivery.state.lock().unwrap().pending.len(), 5);
    }

    #[test]
    fn missing_consumer_and_continuing_producer_remain_bounded() {
        let delivery = delivery(3);
        for _ in 0..1000 {
            delivery.emit(motion(InputSessionId::new(), 1, 1));
            assert!(delivery.state.lock().unwrap().pending.len() <= 3);
        }
        assert!(delivery.overflow.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn waiting_consumer_is_woken_for_input_and_close() {
        let delivery = delivery(2);
        let mut stream = delivery.stream();
        let producer = delivery.clone();
        let event = motion(InputSessionId::new(), 1, 1);
        let expected = event.clone();
        let writer = tokio::spawn(async move {
            tokio::task::yield_now().await;
            producer.emit(event);
        });
        assert_eq!(stream.next().await, Some(expected));
        writer.await.unwrap();
        let producer = delivery.clone();
        let closer = tokio::spawn(async move {
            tokio::task::yield_now().await;
            producer.close();
        });
        assert!(matches!(
            stream.next().await,
            Some(RuntimeEvent::InputDeliveryInvalidated { .. })
        ));
        assert!(stream.next().await.is_none());
        closer.await.unwrap();
    }

    #[tokio::test]
    async fn close_invalidates_pending_inputs_then_ends_stream() {
        let delivery = delivery(2);
        delivery.emit(motion(InputSessionId::new(), 1, 1));
        let mut stream = delivery.stream();
        delivery.close();
        assert!(matches!(
            stream.next().await,
            Some(RuntimeEvent::InputDeliveryInvalidated { .. })
        ));
        assert!(stream.next().await.is_none());
    }
}
