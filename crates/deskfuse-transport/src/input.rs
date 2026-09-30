//! Session-pinned bounded input: reliable transitions and latest-position datagrams.
#[path = "input_motion.rs"]
mod motion;
#[path = "input_update.rs"]
mod update;
use crate::{Envelope, Message, TransportError, read_input_frame, write_frame};
use deskfuse_core::{
    Capabilities, ConnectionContext, DeviceId, InputActivationId, InputSessionId,
    InputShareMessage, MotionDatagram, SharedInputEvent, TransportSessionId,
};
pub use motion::InputMotionDiagnostics;
use motion::{MotionInbox, MotionLink};
use std::{
    collections::{BTreeMap, VecDeque},
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
pub use update::{UpdateTransportRequest, UpdateTransportStream};

#[derive(Clone, Debug)]
pub enum InputTransportEvent {
    Motion {
        peer_id: DeviceId,
        context: ConnectionContext,
        motion: MotionDatagram,
    },
    Message {
        peer_id: DeviceId,
        context: ConnectionContext,
        message: InputShareMessage,
    },
    Invalidated {
        peer_id: DeviceId,
        session_id: TransportSessionId,
        reason: String,
    },
}
pub type InputTransportStream =
    std::pin::Pin<Box<dyn futures::Stream<Item = InputTransportEvent> + Send + 'static>>;
/// Cancellation remains observable even when a task starts waiting after invalidation.
#[derive(Default)]
struct CancelSignal {
    cancelled: AtomicBool,
    wake: tokio::sync::Notify,
}
impl CancelSignal {
    fn notify_waiters(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.wake.notify_waiters();
    }
    async fn notified(&self) {
        let wait = self.wake.notified();
        tokio::pin!(wait);
        wait.as_mut().enable();
        if !self.cancelled.load(Ordering::Acquire) {
            wait.await;
        }
    }
}
#[derive(Clone)]
struct Session {
    negotiated: Capabilities,
    peer: DeviceId,
    connection: quinn::Connection,
    context: ConnectionContext,
    valid: Arc<AtomicBool>,
    cancel: Arc<CancelSignal>,
    motion: Arc<MotionLink>,
}
impl Session {
    fn live(&self) -> bool {
        self.valid.load(Ordering::Acquire)
            && self.connection.close_reason().is_none()
            && self.connection.remote_address() == self.context.remote_addr
            && self
                .connection
                .local_ip()
                .is_none_or(|ip| ip == self.context.local_addr.ip())
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum StreamKey {
    Topology,
    Control(InputSessionId),
    Activation(InputActivationId),
}
fn stream_key(message: &InputShareMessage) -> StreamKey {
    match message {
        InputShareMessage::Activate { activation_id, .. }
        | InputShareMessage::Activated { activation_id, .. }
        | InputShareMessage::Deactivate { activation_id, .. } => {
            StreamKey::Activation(*activation_id)
        }
        _ => message
            .session_id()
            .map_or(StreamKey::Topology, StreamKey::Control),
    }
}
fn requires_motion_capability(message: &InputShareMessage) -> bool {
    matches!(
        message,
        InputShareMessage::RequestHybrid { .. }
            | InputShareMessage::GrantHybrid { .. }
            | InputShareMessage::Hybrid { .. }
    )
}

const INPUT_WRITER_CAPACITY: usize = 128;

/// Ordered transitions with a latest-position slot at each motion boundary.
/// Coalescing happens at enqueue, so a blocked writer cannot accumulate a
/// backlog of obsolete positions. There is exactly one consumer per mailbox.
#[derive(Default)]
struct InputMailbox {
    state: Mutex<InputMailboxState>,
    ready: tokio::sync::Notify,
}
#[derive(Default)]
struct InputMailboxState {
    pending: VecDeque<InputShareMessage>,
    closed: bool,
}
impl InputMailbox {
    fn is_closed(&self) -> bool {
        self.state.lock().map_or(true, |s| s.closed)
    }
    fn close_if_empty(&self) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return true;
        };
        if !state.pending.is_empty() {
            return false;
        }
        state.closed = true;
        true
    }

    fn try_send(&self, mut message: InputShareMessage) -> Result<(), ()> {
        // Frame motion coordinates are absolute; retain only the final
        // position, never discard an event from a mixed transition frame.
        if let InputShareMessage::Frame { events, .. } = &mut message
            && !events.is_empty()
            && events
                .iter()
                .all(|event| matches!(event, SharedInputEvent::Motion { .. }))
        {
            let latest = events.pop().expect("nonempty motion frame");
            events.clear();
            events.push(latest);
        }
        let mut state = self.state.lock().map_err(|_| ())?;
        if state.closed {
            return Err(());
        }
        if let Some(previous) = state.pending.back_mut()
            && can_replace_motion(previous, &message)
        {
            *previous = message;
        } else {
            if state.pending.len() >= INPUT_WRITER_CAPACITY {
                return Err(());
            }
            state.pending.push_back(message);
        }
        drop(state);
        self.ready.notify_one();
        Ok(())
    }

    async fn recv(&self) -> Option<InputShareMessage> {
        loop {
            {
                let mut state = self.state.lock().ok()?;
                if let Some(message) = state.pending.pop_front() {
                    return Some(message);
                }
                if state.closed {
                    return None;
                }
            }
            // notify_one retains a permit if enqueue happens between the
            // empty check and this await; cancellation cannot lose a wakeup.
            self.ready.notified().await;
        }
    }

    fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            state.pending.clear();
        }
        self.ready.notify_one();
    }
}

fn can_replace_motion(previous: &InputShareMessage, next: &InputShareMessage) -> bool {
    match (previous, next) {
        (
            InputShareMessage::Frame {
                session_id: previous_session,
                token: previous_token,
                target_display: previous_display,
                sequence: previous_sequence,
                topology_generation: previous_generation,
                events: previous_events,
            },
            InputShareMessage::Frame {
                session_id,
                token,
                target_display,
                sequence,
                topology_generation,
                events,
            },
        ) => {
            previous_session == session_id
                && previous_token == token
                && previous_display == target_display
                && previous_generation == topology_generation
                // The lease receiver requires increasing, not contiguous,
                // sequences. Never conceal a replay or reorder by merging it.
                && sequence > previous_sequence
                && !previous_events.is_empty()
                && !events.is_empty()
                && previous_events.iter().chain(events).all(|event| {
                    matches!(event, SharedInputEvent::Motion { .. })
                })
        }
        _ => false,
    }
}

type InputWriters = BTreeMap<(TransportSessionId, StreamKey), Arc<InputMailbox>>;
#[derive(Clone)]
pub(crate) struct InputHub {
    updates_enabled: bool,
    executor: Option<tokio::runtime::Handle>,
    background: Option<tokio::runtime::Handle>,
    update_tx: mpsc::Sender<UpdateTransportRequest>,
    update_rx: Arc<Mutex<Option<mpsc::Receiver<UpdateTransportRequest>>>>,
    sessions: Arc<Mutex<BTreeMap<TransportSessionId, Session>>>,
    writers: Arc<Mutex<InputWriters>>,
    tx: mpsc::Sender<InputTransportEvent>,
    motion_inbox: Arc<MotionInbox>,
    rx: Arc<Mutex<Option<mpsc::Receiver<InputTransportEvent>>>>,
}
impl InputHub {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(512);
        let (update_tx, update_rx) = mpsc::channel(32);
        Self {
            executor: crate::input_executor::handle().ok(),
            background: tokio::runtime::Handle::try_current().ok(),
            update_tx,
            update_rx: Arc::new(Mutex::new(Some(update_rx))),
            sessions: Arc::default(),
            writers: Arc::default(),
            tx,
            updates_enabled: true,
            motion_inbox: Arc::default(),
            rx: Arc::new(Mutex::new(Some(rx))),
        }
    }
    pub fn disable_updates(&mut self) {
        self.updates_enabled = false;
    }
    pub fn stream(&self) -> InputTransportStream {
        let rx = self.rx.lock().ok().and_then(|mut rx| rx.take());
        match rx {
            Some(rx) => Box::pin(futures::stream::unfold(
                (rx, self.motion_inbox.clone()),
                |(mut rx, motion)| async move {
                    // Motion never consumes a reliable channel permit. Prioritize
                    // queued state barriers when both channels are ready.
                    let event = tokio::select! {
                        biased;
                        event = rx.recv() => event,
                        event = motion.recv() => Some(event),
                    };
                    event.map(|event| (event, (rx, motion)))
                },
            )),
            None => Box::pin(futures::stream::pending()),
        }
    }
    pub fn contexts(&self, peer: DeviceId) -> Vec<ConnectionContext> {
        self.sessions
            .lock()
            .map(|m| {
                m.values()
                    .filter(|s| s.peer == peer && s.live())
                    .map(|s| s.context.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
    /// Called by the input actor only at a local-cursor boundary, after the new
    /// authenticated route has proved bidirectional datagram readiness.
    pub fn select_route(&self, peer: DeviceId, selected: TransportSessionId) {
        let retired = {
            let Ok(sessions) = self.sessions.lock() else {
                return;
            };
            let Some(chosen) = sessions.get(&selected).filter(|s| {
                s.peer == peer && s.live() && s.context.input_eligible() && s.motion.is_ready()
            }) else {
                return;
            };
            sessions
                .values()
                .filter(|s| s.peer == peer && s.context.session_id != chosen.context.session_id)
                .cloned()
                .collect::<Vec<_>>()
        };
        for session in retired {
            self.invalidate(&session, "input upgraded to preferred physical route");
        }
    }
    pub fn capabilities(&self, peer: DeviceId, id: TransportSessionId) -> Capabilities {
        self.sessions
            .lock()
            .ok()
            .and_then(|sessions| {
                sessions
                    .get(&id)
                    .filter(|session| session.peer == peer && session.live())
                    .map(|session| session.negotiated)
            })
            .unwrap_or_default()
    }
    pub async fn register(
        &self,
        peer: DeviceId,
        connection: quinn::Connection,
        local: SocketAddr,
        interface: String,
        negotiated: Capabilities,
    ) -> bool {
        let Some(executor) = &self.executor else {
            return false;
        };
        if !negotiated.contains(Capabilities::INPUT_SHARING)
            || local.ip().is_unspecified()
            || connection.local_ip().is_some_and(|ip| ip != local.ip())
        {
            return false;
        }
        let remote = connection.remote_address();
        let id = TransportSessionId::new();
        let Ok(context) =
            crate::input_executor::inspect_blocking(move || crate::lan::inspect(id, local, remote))
                .await
        else {
            return false;
        };
        if !context.input_eligible() || context.interface_name.as_deref() != Some(&interface) {
            return false;
        }
        let session = Session {
            negotiated,
            peer,
            connection: connection.clone(),
            context,
            valid: Arc::new(AtomicBool::new(true)),
            cancel: Arc::new(CancelSignal::default()),
            motion: Arc::default(),
        };
        if !self.admit_session(session.clone()) {
            connection.close(crate::QUIC_CLOSE_CODE, b"input session capacity exceeded");
            return false;
        }

        self.start_motion(session.clone());
        if self.updates_enabled {
            self.start_updates(session.clone());
        }
        let hub = self.clone();
        let watched = session.clone();
        let background = self
            .background
            .clone()
            .unwrap_or_else(tokio::runtime::Handle::current);
        background.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(250));
            let reason = loop {
                tokio::select! {
                    _ = interval.tick() => {},
                    _ = watched.cancel.notified() => break "input session cancelled",
                }
                if !watched.live() {
                    break "input QUIC connection closed";
                }
                let original = watched.context.clone();
                let Ok(current) = crate::input_executor::inspect_blocking(move || {
                    crate::lan::inspect(
                        original.session_id,
                        original.local_addr,
                        original.remote_addr,
                    )
                })
                .await
                else {
                    break "input route inspection failed";
                };
                if current.interface_name != watched.context.interface_name {
                    break "input route interface changed";
                }
                if current.interface_kind != watched.context.interface_kind {
                    break "input route kind changed";
                }
                if current.scope != watched.context.scope {
                    break "input route scope changed";
                }
            };
            hub.invalidate(&watched, reason);
        });
        let hub = self.clone();
        executor.spawn(async move {
            loop {
                let incoming = tokio::select! {stream=connection.accept_uni()=>stream,_=session.cancel.notified()=>break};
                let Ok(mut recv) = incoming else { break };
                if !session.live() {
                    break;
                }
                let hub = hub.clone();
                let session = session.clone();
                tokio::spawn(async move {
                    let mut stream_session = StreamKey::Topology;
                    let mut first = true;
                    loop {
                        let frame = tokio::select! {frame=read_input_frame(&mut recv,Duration::from_secs(3))=>frame,_=session.cancel.notified()=>break};
                        let Ok(frame) = frame else { break };
                        let Some(Message::InputSharing(wire)) = frame.message else {
                            break;
                        };
                        let Ok(message) = wire.to_core() else { break };
                        if matches!(stream_key(&message), StreamKey::Activation(_))
                            && !session.negotiated.contains(Capabilities::INPUT_ACTIVATION)
                        {
                            hub.invalidate(&session, "activation capability was not negotiated");
                            break;
                        }
                        if requires_motion_capability(&message)
                            && !session
                                .negotiated
                                .contains(Capabilities::INPUT_MOTION_DATAGRAM_V1)
                        {
                            hub.invalidate(&session, "motion capability was not negotiated");
                            break;
                        }
                        if !session.live() || !session.context.input_eligible() {
                            hub.invalidate(&session, "input route no longer eligible");
                            break;
                        }
                        if first {
                            stream_session = stream_key(&message);
                            first = false;
                        } else if stream_session != stream_key(&message) {
                            hub.invalidate(&session, "input stream changed session");
                            break;
                        }
                        if hub
                            .tx
                            .try_send(InputTransportEvent::Message {
                                peer_id: session.peer,
                                context: session.context.clone(),
                                message,
                            })
                            .is_err()
                        {
                            hub.invalidate(&session, "input receive queue overflow");
                            break;
                        }
                    }
                });
            }
        });
        true
    }
    fn admit_session(&self, session: Session) -> bool {
        let Ok(mut sessions) = self.sessions.lock() else {
            return false;
        };
        // Two simultaneous dial directions are valid; repeated connections
        // must not multiply route watchers and QUIC tasks without a bound.
        if sessions.len() >= 64
            || sessions.values().filter(|s| s.peer == session.peer).count() >= 4
            || sessions
                .values()
                .filter(|s| {
                    s.peer == session.peer
                        && s.context.local_addr.ip() == session.context.local_addr.ip()
                })
                .count()
                >= 2
        {
            return false;
        }
        sessions.insert(session.context.session_id, session);
        true
    }
    fn invalidate(&self, session: &Session, reason: &'static str) {
        if session.valid.swap(false, Ordering::AcqRel) {
            session.motion.sample_diagnostics(session, reason, true);
            session.cancel.notify_waiters();
            session.motion.close();
            self.motion_inbox
                .remove_transport(session.context.session_id);
            session
                .connection
                .close(crate::QUIC_CLOSE_CODE, b"input session invalidated");
            if let Ok(mut sessions) = self.sessions.lock() {
                sessions.remove(&session.context.session_id);
            }
            if let Ok(mut writers) = self.writers.lock() {
                writers.retain(|(id, _), _| *id != session.context.session_id);
            }
            let _ = self.tx.try_send(InputTransportEvent::Invalidated {
                peer_id: session.peer,
                session_id: session.context.session_id,
                reason: reason.into(),
            });
        }
    }
    pub fn send(
        &self,
        peer: DeviceId,
        id: TransportSessionId,
        message: InputShareMessage,
    ) -> Result<(), TransportError> {
        message
            .validate()
            .map_err(|_| TransportError::MalformedPeer("invalid input message"))?;
        let session = self
            .sessions
            .lock()
            .map_err(|_| TransportError::Closed)?
            .get(&id)
            .filter(|s| s.peer == peer && s.live() && s.context.input_eligible())
            .cloned()
            .ok_or(TransportError::Closed)?;
        let message_key = stream_key(&message);
        if matches!(message_key, StreamKey::Activation(_))
            && !session.negotiated.contains(Capabilities::INPUT_ACTIVATION)
        {
            return Err(TransportError::MalformedPeer(
                "activation capability was not negotiated",
            ));
        }
        if requires_motion_capability(&message)
            && !session
                .negotiated
                .contains(Capabilities::INPUT_MOTION_DATAGRAM_V1)
        {
            return Err(TransportError::MalformedPeer(
                "motion capability was not negotiated",
            ));
        }
        if let InputShareMessage::Leave { session_id, .. }
        | InputShareMessage::Release { session_id, .. }
        | InputShareMessage::Denied { session_id, .. } = &message
        {
            self.cancel_motion(peer, id, *session_id);
        }
        let key = (id, message_key);
        let mut writers = self.writers.lock().map_err(|_| TransportError::Closed)?;
        if writers.get(&key).is_some_and(|writer| writer.is_closed()) {
            writers.remove(&key);
        }
        let tx = writers
            .entry(key)
            .or_insert_with(|| self.spawn_writer(key, session.clone()))
            .clone();
        if tx.try_send(message.clone()).is_err() {
            // Idle retirement is normal: replace the closed mailbox under the map lock.
            if tx.is_closed() {
                let replacement = self.spawn_writer(key, session.clone());
                let sent = replacement.try_send(message);
                writers.insert(key, replacement);
                if sent.is_ok() {
                    return Ok(());
                }
            }
            drop(writers);
            self.invalidate(&session, "input send queue overflow");
            return Err(TransportError::Closed);
        }
        Ok(())
    }
    fn spawn_writer(
        &self,
        key: (TransportSessionId, StreamKey),
        session: Session,
    ) -> Arc<InputMailbox> {
        let tx = Arc::new(InputMailbox::default());
        let rx = tx.clone();
        let hub = self.clone();
        let Some(executor) = &self.executor else {
            tx.close();
            return tx;
        };
        executor.spawn(async move {
            let Ok(Ok(mut stream)) =
                tokio::time::timeout(Duration::from_millis(500), session.connection.open_uni())
                    .await
            else {
                rx.close();
                hub.invalidate(&session, "input stream could not open");
                return;
            };
            // Set before the first write: state anchors/clicks must not wait for
            // lower-priority update payloads on this same LAN connection.
            if stream.set_priority(i32::MAX).is_err() {
                rx.close();
                hub.invalidate(&session, "input stream priority unavailable");
                return;
            }
            loop {
                let next = tokio::select! {next=tokio::time::timeout(Duration::from_secs(3),rx.recv())=>next,_=session.cancel.notified()=>break};
                let message = match next {
                    Ok(Some(message)) => message,
                    Err(_) if !rx.close_if_empty() => continue,
                    _ => {
                        let _ = stream.finish();
                        break;
                    }
                };
                if !session.live() {
                    break;
                }
                let release = matches!(
                    message,
                    InputShareMessage::Release { .. }
                        | InputShareMessage::Denied { .. }
                        | InputShareMessage::Deactivate { .. }
                );
                let Ok(wire) = deskfuse_protocol::InputSharing::from_core(&message) else {
                    break;
                };
                let envelope = Envelope::new([0; 16], Message::InputSharing(wire));
                match tokio::time::timeout(
                    Duration::from_millis(500),
                    write_frame(&mut stream, &envelope),
                )
                .await
                {
                    Ok(Ok(())) => {}
                    _ => {
                        hub.invalidate(&session, "input stream stalled");
                        break;
                    }
                }
                if release {
                    let _ = stream.finish();
                    break;
                }
            }
            rx.close();
            if let Ok(mut writers) = hub.writers.lock()
                && writers
                    .get(&key)
                    .is_some_and(|current| Arc::ptr_eq(current, &rx))
                {
                    writers.remove(&key);
                }
        });
        tx
    }
    pub fn invalidate_peer(&self, peer: DeviceId) {
        let sessions = self
            .sessions
            .lock()
            .map(|s| {
                s.values()
                    .filter(|s| s.peer == peer)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for session in sessions {
            self.invalidate(&session, "peer disconnected or revoked");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deskfuse_core::{InputToken, InterfaceKind, LinkScope};
    fn motion(sequence: u64) -> InputShareMessage {
        InputShareMessage::Frame {
            session_id: InputSessionId::new(),
            token: InputToken([9; 32]),
            target_display: deskfuse_core::DisplayId("test-display".into()),
            sequence,
            topology_generation: 1,
            events: vec![SharedInputEvent::Motion {
                x: sequence as f64,
                y: 12.0,
            }],
        }
    }

    fn with_sequence(mut message: InputShareMessage, next: u64) -> InputShareMessage {
        if let InputShareMessage::Frame {
            sequence, events, ..
        } = &mut message
        {
            *sequence = next;
            *events = vec![SharedInputEvent::Motion {
                x: next as f64,
                y: 12.0,
            }];
        }
        message
    }

    #[tokio::test]
    async fn motion_burst_retains_only_latest_position_at_enqueue() {
        let mailbox = InputMailbox::default();
        let first = motion(1);
        for sequence in 1..=10_000 {
            mailbox
                .try_send(with_sequence(first.clone(), sequence))
                .unwrap();
        }
        assert_eq!(mailbox.state.lock().unwrap().pending.len(), 1);
        assert_eq!(mailbox.recv().await, Some(with_sequence(first, 10_000)));
    }

    #[tokio::test]
    async fn motion_only_batch_is_compacted_to_its_final_absolute_position() {
        let mailbox = InputMailbox::default();
        let expected = motion(7);
        let mut batch = expected.clone();
        if let InputShareMessage::Frame { events, .. } = &mut batch {
            events.insert(0, SharedInputEvent::Motion { x: 3.0, y: 4.0 });
            events.insert(0, SharedInputEvent::Motion { x: 1.0, y: 2.0 });
        }
        mailbox.try_send(batch).unwrap();
        assert_eq!(mailbox.recv().await, Some(expected));
    }

    #[tokio::test]
    async fn motion_never_coalesces_across_ordered_transitions() {
        let first = motion(1);
        let InputShareMessage::Frame {
            session_id, token, ..
        } = first.clone()
        else {
            unreachable!()
        };
        let mut barriers = vec![
            InputShareMessage::Renew {
                session_id,
                token,
                sequence: 2,
            },
            InputShareMessage::Leave {
                session_id,
                token,
                sequence: 2,
            },
            InputShareMessage::Release {
                session_id,
                token,
                sequence: 2,
            },
            InputShareMessage::Enter {
                session_id,
                token,
                sequence: 2,
                target_display: deskfuse_core::DisplayId("test-display".into()),
                x: 2.0,
                y: 12.0,
            },
        ];
        for event in [
            SharedInputEvent::Button {
                button: 0,
                pressed: true,
            },
            SharedInputEvent::Button {
                button: 0,
                pressed: false,
            },
            SharedInputEvent::Key {
                code: 12,
                pressed: true,
                modifiers: 0,
                repeat: false,
            },
            SharedInputEvent::Key {
                code: 12,
                pressed: false,
                modifiers: 0,
                repeat: false,
            },
            SharedInputEvent::Scroll { dx: 1.0, dy: 2.0 },
            SharedInputEvent::Modifiers { modifiers: 1 },
        ] {
            let mut barrier = with_sequence(first.clone(), 2);
            if let InputShareMessage::Frame { events, .. } = &mut barrier {
                events.push(event);
            }
            barriers.push(barrier);
        }
        for barrier in barriers {
            let mailbox = InputMailbox::default();
            let last = with_sequence(first.clone(), 4);
            for message in [
                first.clone(),
                barrier.clone(),
                with_sequence(first.clone(), 3),
                last.clone(),
            ] {
                mailbox.try_send(message).unwrap();
            }
            assert_eq!(mailbox.state.lock().unwrap().pending.len(), 3);
            assert_eq!(mailbox.recv().await, Some(first.clone()));
            assert_eq!(mailbox.recv().await, Some(barrier));
            assert_eq!(mailbox.recv().await, Some(last));
        }
    }

    #[tokio::test]
    async fn motion_preserves_session_token_display_topology_and_sequence_boundaries() {
        let first = motion(2);
        for boundary in 0..6 {
            let mailbox = InputMailbox::default();
            let mut next = with_sequence(first.clone(), 3);
            if let InputShareMessage::Frame {
                session_id,
                token,
                target_display,
                topology_generation,
                sequence,
                ..
            } = &mut next
            {
                match boundary {
                    0 => *session_id = InputSessionId::new(),
                    1 => *token = InputToken::new(),
                    2 => target_display.0 = "other-display".into(),
                    3 => *topology_generation += 1,
                    4 => *sequence = 2,
                    _ => *sequence = 1,
                }
            }
            mailbox.try_send(first.clone()).unwrap();
            mailbox.try_send(next.clone()).unwrap();
            assert_eq!(mailbox.state.lock().unwrap().pending.len(), 2);
            assert_eq!(mailbox.recv().await, Some(first.clone()));
            assert_eq!(mailbox.recv().await, Some(next));
        }
    }

    #[tokio::test]
    async fn mailbox_bounds_transitions_but_updates_latest_motion_when_full() {
        let mailbox = InputMailbox::default();
        let first = motion(1);
        let InputShareMessage::Frame {
            session_id, token, ..
        } = first.clone()
        else {
            unreachable!()
        };
        for sequence in 1..INPUT_WRITER_CAPACITY as u64 {
            mailbox
                .try_send(InputShareMessage::Renew {
                    session_id,
                    token,
                    sequence,
                })
                .unwrap();
        }
        mailbox.try_send(with_sequence(first.clone(), 128)).unwrap();
        mailbox.try_send(with_sequence(first.clone(), 129)).unwrap();
        assert_eq!(
            mailbox.state.lock().unwrap().pending.len(),
            INPUT_WRITER_CAPACITY
        );
        assert!(
            mailbox
                .try_send(InputShareMessage::Release {
                    session_id,
                    token,
                    sequence: 130
                })
                .is_err()
        );
        for _ in 1..INPUT_WRITER_CAPACITY {
            assert!(matches!(
                mailbox.recv().await,
                Some(InputShareMessage::Renew { .. })
            ));
        }
        assert_eq!(mailbox.recv().await, Some(with_sequence(first, 129)));
    }

    #[tokio::test]
    async fn mailbox_wakes_after_empty_wait_cancel_and_close() {
        let mailbox = Arc::new(InputMailbox::default());
        assert!(
            tokio::time::timeout(Duration::from_millis(1), mailbox.recv())
                .await
                .is_err()
        );
        let consumer = mailbox.clone();
        let waiting = tokio::spawn(async move { consumer.recv().await });
        tokio::task::yield_now().await;
        let message = motion(1);
        mailbox.try_send(message.clone()).unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), waiting)
                .await
                .unwrap()
                .unwrap(),
            Some(message.clone())
        );
        let consumer = mailbox.clone();
        let waiting = tokio::spawn(async move { consumer.recv().await });
        tokio::task::yield_now().await;
        mailbox.close();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), waiting)
                .await
                .unwrap()
                .unwrap(),
            None
        );
        assert!(mailbox.try_send(message).is_err());
    }

    #[tokio::test]
    async fn idle_retirement_preserves_a_message_that_wins_the_race() {
        let mailbox = InputMailbox::default();
        let message = motion(1);
        mailbox.try_send(message.clone()).unwrap();
        assert!(!mailbox.close_if_empty());
        assert_eq!(mailbox.recv().await, Some(message));
        assert!(mailbox.close_if_empty());
        assert!(mailbox.try_send(motion(2)).is_err());
        let replacement = InputMailbox::default();
        replacement.try_send(motion(3)).unwrap();
        assert!(!replacement.is_closed());
    }
    #[tokio::test]
    async fn cancellation_is_remembered_for_late_readers() {
        let signal = CancelSignal::default();
        signal.notify_waiters();
        tokio::time::timeout(Duration::from_millis(50), signal.notified())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_millis(50), signal.notified())
            .await
            .unwrap();
    }
    pub(super) async fn connections() -> (
        quinn::Endpoint,
        quinn::Endpoint,
        quinn::Connection,
        quinn::Connection,
    ) {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let der = cert.cert.der().to_vec();
        let key = cert.key_pair.serialize_der();
        let placeholder = "127.0.0.1:0".parse().unwrap();
        let config = crate::QuinnTransportConfig::new(
            placeholder,
            placeholder,
            "localhost",
            der.clone(),
            key,
            der,
        );
        let (server, client) = crate::build_input_quic_configs(&config).unwrap();
        let server = quinn::Endpoint::server(server, placeholder).unwrap();
        let mut client_endpoint = quinn::Endpoint::client(placeholder).unwrap();
        client_endpoint.set_default_client_config(client);
        let outgoing = client_endpoint
            .connect(server.local_addr().unwrap(), "localhost")
            .unwrap();
        let incoming = server.accept();
        let (a, b) = tokio::join!(outgoing, async { incoming.await.unwrap().await.unwrap() });
        (server, client_endpoint, a.unwrap(), b)
    }
    #[tokio::test]
    async fn full_size_wallpaper_crosses_the_input_stream() {
        let (_server, client, outgoing, incoming) = connections().await;
        let hub = InputHub::new();
        let peer = DeviceId::from_bytes([7; 32]);
        let context = ConnectionContext {
            session_id: TransportSessionId::new(),
            local_addr: client.local_addr().unwrap(),
            remote_addr: outgoing.remote_address(),
            interface_name: None,
            interface_kind: InterfaceKind::Loopback,
            scope: LinkScope::Unknown,
        };
        let session = Session {
            negotiated: Capabilities::INPUT_SHARING,
            peer,
            connection: outgoing,
            context: context.clone(),
            valid: Arc::new(AtomicBool::new(true)),
            cancel: Arc::new(CancelSignal::default()),
            motion: Arc::default(),
        };
        let message = InputShareMessage::DisplayWallpaper {
            display_id: deskfuse_core::DisplayId::new("screen"),
            jpeg: [
                vec![0xff, 0xd8, 0xff],
                vec![42; deskfuse_core::MAX_DISPLAY_WALLPAPER_BYTES - 3],
            ]
            .concat(),
        };
        let wire = deskfuse_protocol::InputSharing::from_core(&message).unwrap();
        assert!(wire.payload.len() > 64 * 1024);
        let writer = hub.spawn_writer((context.session_id, StreamKey::Topology), session);
        writer.try_send(message.clone()).unwrap();
        let mut stream = tokio::time::timeout(Duration::from_secs(2), incoming.accept_uni())
            .await
            .unwrap()
            .unwrap();
        let frame = read_input_frame(&mut stream, Duration::from_secs(2))
            .await
            .unwrap();
        let Some(Message::InputSharing(received)) = frame.message else {
            panic!("wrong frame")
        };
        assert_eq!(received.to_core().unwrap(), message);
    }

    #[tokio::test]
    async fn persistent_writer_orders_frames_and_releases_stream_capacity() {
        // Socket/route authorization is tested separately. Exercise the exact
        // production writer on loopback, below that policy boundary.
        let (_server, client, a, b) = connections().await;
        let hub = InputHub::new();
        let peer = DeviceId::from_bytes([7; 32]);
        let context = ConnectionContext {
            session_id: TransportSessionId::new(),
            local_addr: client.local_addr().unwrap(),
            remote_addr: a.remote_address(),
            interface_name: None,
            interface_kind: InterfaceKind::Loopback,
            scope: LinkScope::Unknown,
        };
        let session = Session {
            negotiated: Capabilities::INPUT_SHARING | Capabilities::INPUT_ACTIVATION,
            peer,
            connection: a,
            context: context.clone(),
            valid: Arc::new(AtomicBool::new(true)),
            cancel: Arc::new(CancelSignal::default()),
            motion: Arc::default(),
        };
        for _ in 0..70 {
            let id = InputSessionId::new();
            let key = (context.session_id, StreamKey::Control(id));
            let tx = hub.spawn_writer(key, session.clone());
            hub.writers.lock().unwrap().insert(key, tx.clone());
            let token = InputToken::new();
            for sequence in 1..=10 {
                tx.try_send(InputShareMessage::Renew {
                    session_id: id,
                    token,
                    sequence,
                })
                .unwrap();
            }
            tx.try_send(InputShareMessage::Release {
                session_id: id,
                token,
                sequence: 11,
            })
            .unwrap();
            let mut stream = tokio::time::timeout(Duration::from_secs(2), b.accept_uni())
                .await
                .unwrap()
                .unwrap();
            for expected in 1..=11 {
                let frame = read_input_frame(&mut stream, Duration::from_secs(2))
                    .await
                    .unwrap();
                let Some(Message::InputSharing(wire)) = frame.message else {
                    panic!("wrong frame")
                };
                let sequence = match wire.to_core().unwrap() {
                    InputShareMessage::Renew { sequence, .. }
                    | InputShareMessage::Release { sequence, .. } => sequence,
                    _ => panic!("wrong message"),
                };
                assert_eq!(sequence, expected);
            }
            tokio::task::yield_now().await;
            assert!(!hub.writers.lock().unwrap().contains_key(&key));
        }
        for _ in 0..70 {
            let activation_id = InputActivationId::new();
            let intent_id = InputActivationId::new();
            let key = (context.session_id, StreamKey::Activation(activation_id));
            let tx = hub.spawn_writer(key, session.clone());
            hub.writers.lock().unwrap().insert(key, tx.clone());
            for sequence in 1..=3 {
                tx.try_send(InputShareMessage::Activate {
                    activation_id,
                    intent_id,
                    ttl_ms: 1500,
                    sequence,
                    force: false,
                })
                .unwrap();
            }
            tx.try_send(InputShareMessage::Deactivate {
                activation_id,
                sequence: 4,
            })
            .unwrap();
            let mut stream = tokio::time::timeout(Duration::from_secs(2), b.accept_uni())
                .await
                .unwrap()
                .unwrap();
            for expected in 1..=4 {
                let frame = read_input_frame(&mut stream, Duration::from_secs(2))
                    .await
                    .unwrap();
                let Some(Message::InputSharing(wire)) = frame.message else {
                    panic!("wrong frame")
                };
                let message = wire.to_core().unwrap();
                assert_eq!(stream_key(&message), StreamKey::Activation(activation_id));
                let sequence = match message {
                    InputShareMessage::Activate { sequence, .. }
                    | InputShareMessage::Deactivate { sequence, .. } => sequence,
                    _ => panic!("wrong activation"),
                };
                assert_eq!(sequence, expected);
            }
            tokio::task::yield_now().await;
            assert!(!hub.writers.lock().unwrap().contains_key(&key));
        }
        assert!(hub.writers.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn activation_capabilities_belong_to_exact_negotiated_session() {
        let (_server, client, a, _b) = connections().await;
        let hub = InputHub::new();
        let peer = DeviceId::from_bytes([5; 32]);
        let mut ids = Vec::new();
        for negotiated in [
            Capabilities::INPUT_SHARING,
            Capabilities::INPUT_SHARING | Capabilities::INPUT_ACTIVATION,
        ] {
            let id = TransportSessionId::new();
            ids.push(id);
            let context = ConnectionContext {
                session_id: id,
                local_addr: client.local_addr().unwrap(),
                remote_addr: a.remote_address(),
                interface_name: None,
                interface_kind: InterfaceKind::Loopback,
                scope: LinkScope::Unknown,
            };
            assert!(hub.admit_session(Session {
                negotiated,
                peer,
                connection: a.clone(),
                context,
                valid: Arc::new(AtomicBool::new(true)),
                cancel: Arc::new(CancelSignal::default()),
                motion: Arc::default(),
            }));
        }
        assert!(
            !hub.capabilities(peer, ids[0])
                .contains(Capabilities::INPUT_ACTIVATION)
        );
        assert!(
            hub.capabilities(peer, ids[1])
                .contains(Capabilities::INPUT_ACTIVATION)
        );
        assert_eq!(
            hub.capabilities(DeviceId::from_bytes([6; 32]), ids[1]),
            Capabilities::default()
        );
    }
    #[tokio::test]
    async fn fake_discovery_capability_cannot_register_signed_legacy_hello() {
        let (_server, client, a, _b) = connections().await;
        let hub = InputHub::new();
        let peer = DeviceId::from_bytes([2; 32]);
        assert!(
            !hub.register(
                peer,
                a,
                client.local_addr().unwrap(),
                "en0".into(),
                Capabilities::NETWORK
            )
            .await
        );
        assert!(hub.contexts(peer).is_empty());
    }
    #[tokio::test]
    async fn loopback_and_unbound_sources_never_get_input_contexts() {
        let (_server, _client, a, _b) = connections().await;
        let hub = InputHub::new();
        assert!(
            !hub.register(
                DeviceId::from_bytes([2; 32]),
                a,
                "0.0.0.0:42".parse().unwrap(),
                "en0".into(),
                Capabilities::INPUT_SHARING
            )
            .await
        );
        assert!(hub.sessions.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn repeated_authenticated_connections_do_not_grow_input_registry() {
        let (_server, client, a, _b) = connections().await;
        let hub = InputHub::new();
        let peer = DeviceId::from_bytes([44; 32]);
        for index in 0..20 {
            let context = ConnectionContext {
                session_id: TransportSessionId::new(),
                local_addr: client.local_addr().unwrap(),
                remote_addr: a.remote_address(),
                interface_name: None,
                interface_kind: InterfaceKind::Loopback,
                scope: LinkScope::Unknown,
            };
            let session = Session {
                negotiated: Capabilities::INPUT_SHARING | Capabilities::INPUT_ACTIVATION,
                peer,
                connection: a.clone(),
                context,
                valid: Arc::new(AtomicBool::new(true)),
                cancel: Arc::new(CancelSignal::default()),
                motion: Arc::default(),
            };
            assert_eq!(hub.admit_session(session), index < 2);
        }
        assert_eq!(hub.sessions.lock().unwrap().len(), 2);
    }
}
