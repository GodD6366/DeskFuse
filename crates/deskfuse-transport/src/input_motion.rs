//! A single QUIC DATAGRAM reader/writer pair per authenticated input connection.
//! This module transports positions only; token, epoch and display authorization
//! remain the runtime's responsibility. Buffers cannot grow with motion frequency.
use super::{InputHub, InputTransportEvent, Session};
use crate::TransportError;
use deskfuse_core::{
    Capabilities, ConnectionContext, DeviceId, InputSessionId, InputStage, InputToken,
    MotionDatagram, QualityObservation, RelativeMotionDatagram, TransportSessionId,
    note_quality_tick, record_input_stage, relative_delta_ok,
};
use deskfuse_protocol::{
    MAX_MOTION_DATAGRAM_SIZE, decode_motion_datagram, decode_relative_motion_datagram,
    encode_motion_datagram, encode_relative_motion_datagram,
};
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::Notify;

const MAX_CONTROL_SLOTS: usize = 8;
const PROBE_INTERVAL: Duration = Duration::from_millis(250);
/// 质量窗口的独立采样节奏。`sample_diagnostics` 自身再按一秒节流。
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const PROBE_TIMEOUT: Duration = Duration::from_millis(1500);
const MAX_RECEIVE_SLOTS: usize = 512; // Increased from 128 to reduce event replacement
// Disjoint from the protocol's motion framing; exact length and nonce are checked.
const PROBE_MAGIC: &[u8; 8] = b"DFPROBE1";
const PROBE_SIZE: usize = 41;
const PROBE_PING: u8 = 1;
const PROBE_PONG: u8 = 2;

/// Metadata only: no credentials, coordinates, display identifiers or input bodies.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InputMotionDiagnostics {
    pub available: bool,
    pub sent: u64,
    pub received: u64,
    pub send_replaced: u64,
    pub receive_replaced: u64,
    pub dropped: u64,
    pub pending_send: usize,
    pub rtt_micros: u64,
    pub lost_packets: u64,
    pub sent_packets: u64,
    pub congestion_events: u64,
}

/// Challenge rotation prevents delayed/duplicated old pongs from keeping a dead
/// datagram path ready. Only an outstanding challenge is acknowledged once.
struct ProbeState {
    nonce: [u8; 32],
    issued: tokio::time::Instant,
    awaiting: bool,
    last_pong: Option<tokio::time::Instant>,
}
impl Default for ProbeState {
    fn default() -> Self {
        Self {
            nonce: InputToken::new().0,
            issued: tokio::time::Instant::now(),
            awaiting: false,
            last_pong: None,
        }
    }
}
impl ProbeState {
    fn challenge(&mut self) -> [u8; 32] {
        if !self.awaiting || self.issued.elapsed() >= PROBE_TIMEOUT {
            self.nonce = InputToken::new().0;
            self.issued = tokio::time::Instant::now();
            self.awaiting = true;
        }
        self.nonce
    }
    fn acknowledge(&mut self, nonce: [u8; 32]) -> bool {
        if !self.awaiting || nonce != self.nonce || self.issued.elapsed() >= PROBE_TIMEOUT {
            return false;
        }
        self.awaiting = false;
        self.last_pong = Some(tokio::time::Instant::now());
        true
    }
    fn healthy(&self) -> bool {
        self.last_pong
            .is_some_and(|at| at.elapsed() < PROBE_TIMEOUT)
    }
}

#[derive(Default)]
pub(super) struct MotionLink {
    ready: AtomicBool,
    // Setting started protects against accidentally installing competing readers.
    started: AtomicBool,
    sampling_started: AtomicBool,
    probe: Mutex<ProbeState>,
    pending: Mutex<BTreeMap<InputSessionId, MotionDatagram>>,
    pending_relative: Mutex<BTreeMap<InputSessionId, RelativeMotionDatagram>>,
    wake: Notify,
    sent: AtomicU64,
    received: AtomicU64,
    send_replaced: AtomicU64,
    receive_replaced: AtomicU64,
    dropped: AtomicU64,
    /// Last diagnostics sample, so an export gets a steady trace without the
    /// motion path paying for a sample per position.
    diag_at: Mutex<Option<Instant>>,
    quality_counters: Mutex<Option<(u64, u64, u64)>>,
}
impl MotionLink {
    pub(super) fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }
    /// Sample the datagram path for the diagnostics export.
    ///
    /// At most one sample per second per link, because `stats()` takes the QUIC
    /// connection lock; a `force` sample fires regardless, so the moment the
    /// path is declared unusable is captured while debug collection is enabled.
    pub(super) fn sample_diagnostics(&self, session: &Session, event: &'static str, force: bool) {
        let now = Instant::now();
        let Ok(mut last) = self.diag_at.try_lock() else {
            return;
        };
        if !force
            && last.is_some_and(|at| now.saturating_duration_since(at) < Duration::from_secs(1))
        {
            return;
        }
        *last = Some(now);
        drop(last);
        let (Ok(pending), Ok(relative)) =
            (self.pending.try_lock(), self.pending_relative.try_lock())
        else {
            return;
        };
        let pending_send = pending.len() + relative.len();
        drop(pending);
        drop(relative);
        let path = session.connection.stats().path;
        let dropped = self.dropped.load(Ordering::Relaxed);
        let lost_packets = path.lost_packets;
        let congestion_events = path.congestion_events;
        let hitches = if let Ok(mut previous) = self.quality_counters.try_lock() {
            let current = (dropped, lost_packets, congestion_events);
            let hitches = previous.map_or(0, |old| {
                current
                    .0
                    .saturating_sub(old.0)
                    .saturating_add(current.1.saturating_sub(old.1))
                    .saturating_add(current.2.saturating_sub(old.2))
            });
            *previous = Some(current);
            hitches.min(u32::MAX as u64) as u32
        } else {
            0
        };
        let rtt_micros = path.rtt.as_micros().min(u128::from(u64::MAX)) as u64;
        // 先折叠这一秒的阶段计数再记录：慢注入次数只能从这个返回值拿到，
        // 它已经从原子计数里取走了。
        let tick = note_quality_tick(QualityObservation {
            timestamp_ms: crate::motion_diag::timestamp_ms().min(u128::from(u64::MAX)) as u64,
            rtt_micros,
            hitches,
            pending_send: u32::try_from(pending_send).unwrap_or(u32::MAX),
            sent: self.sent.load(Ordering::Relaxed),
            dropped,
            replaced: self.send_replaced.load(Ordering::Relaxed),
        });
        crate::motion_diag::record_quality(crate::motion_diag::QualityRecord {
            timestamp_ms: crate::motion_diag::timestamp_ms(),
            rtt_micros,
            hitches,
            slow_injections: tick.slow_injections,
            capture_queue_micros: tick.capture_queue_micros,
            capture_count: tick.capture_count,
            ready: self.ready.load(Ordering::Acquire),
        });
        if !deskfuse_core::debug_logging::enabled() {
            return;
        }
        crate::motion_diag::record(crate::motion_diag::MotionDiagRecord {
            timestamp_ms: crate::motion_diag::timestamp_ms(),
            event,
            peer: crate::motion_diag::short_peer(session.peer),
            transport_session: crate::motion_diag::short_session(session.context.session_id),
            local_addr: session.context.local_addr.to_string(),
            remote_addr: session.context.remote_addr.to_string(),
            interface_name: session.context.interface_name.clone(),
            interface_kind: format!("{:?}", session.context.interface_kind),
            ready: self.ready.load(Ordering::Acquire),
            sent: self.sent.load(Ordering::Relaxed),
            received: self.received.load(Ordering::Relaxed),
            send_replaced: self.send_replaced.load(Ordering::Relaxed),
            receive_replaced: self.receive_replaced.load(Ordering::Relaxed),
            dropped,
            pending_send,
            rtt_micros: path.rtt.as_micros().min(u128::from(u64::MAX)) as u64,
            sent_packets: path.sent_packets,
            lost_packets,
            congestion_events,
        });
    }
    pub(super) fn close(&self) {
        self.ready.store(false, Ordering::Release);
        if let Ok(mut pending) = self.pending.lock() {
            pending.clear();
        }
        if let Ok(mut pending) = self.pending_relative.lock() {
            pending.clear();
        }
        self.wake.notify_one();
    }
    fn enqueue(&self, motion: MotionDatagram) -> Result<(), TransportError> {
        let mut pending = self.pending.lock().map_err(|_| TransportError::Closed)?;
        if let Some(previous) = pending.get(&motion.session_id) {
            if previous.token == motion.token
                && (previous.position.motion_seq >= motion.position.motion_seq
                    || previous.state_epoch > motion.state_epoch)
            {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
            self.send_replaced.fetch_add(1, Ordering::Relaxed);
        } else if pending.len() >= MAX_CONTROL_SLOTS {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return Err(TransportError::Closed);
        }
        pending.insert(motion.session_id, motion);
        drop(pending);
        self.wake.notify_one();
        Ok(())
    }

    /// 同一会话累加相对计数。超上限时丢掉该槽，不发出非法大包。
    fn enqueue_relative(&self, motion: RelativeMotionDatagram) -> Result<(), TransportError> {
        let mut pending = self
            .pending_relative
            .lock()
            .map_err(|_| TransportError::Closed)?;
        if let Some(previous) = pending.get(&motion.session_id).cloned() {
            if previous.token == motion.token && previous.state_epoch == motion.state_epoch {
                if motion.motion_seq <= previous.motion_seq {
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                    return Ok(());
                }
                let dx = previous.dx + motion.dx;
                let dy = previous.dy + motion.dy;
                if !relative_delta_ok(dx) || !relative_delta_ok(dy) {
                    pending.remove(&motion.session_id);
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                    self.send_replaced.fetch_add(1, Ordering::Relaxed);
                    drop(pending);
                    self.wake.notify_one();
                    return Ok(());
                }
                pending.insert(
                    motion.session_id,
                    RelativeMotionDatagram {
                        dx,
                        dy,
                        motion_seq: motion.motion_seq,
                        ..motion
                    },
                );
                drop(pending);
                self.wake.notify_one();
                return Ok(());
            }
            if previous.token == motion.token && previous.state_epoch > motion.state_epoch {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
            self.send_replaced.fetch_add(1, Ordering::Relaxed);
        } else if pending.len() >= MAX_CONTROL_SLOTS {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return Err(TransportError::Closed);
        }
        pending.insert(motion.session_id, motion);
        drop(pending);
        self.wake.notify_one();
        Ok(())
    }

    fn pop_outgoing(&self) -> Option<OutgoingMotion> {
        if let Ok(mut pending) = self.pending.lock()
            && let Some((_, motion)) = pending.pop_first()
        {
            return Some(OutgoingMotion::Absolute(motion));
        }
        if let Ok(mut pending) = self.pending_relative.lock()
            && let Some((_, motion)) = pending.pop_first()
        {
            return Some(OutgoingMotion::Relative(motion));
        }
        None
    }

    fn has_outgoing(&self) -> bool {
        self.pending.lock().is_ok_and(|pending| !pending.is_empty())
            || self
                .pending_relative
                .lock()
                .is_ok_and(|pending| !pending.is_empty())
    }
}

enum OutgoingMotion {
    Absolute(MotionDatagram),
    Relative(RelativeMotionDatagram),
}

struct ReceivedMotion {
    peer: DeviceId,
    context: ConnectionContext,
    motion: MotionDatagram,
}
struct ReceivedRelative {
    peer: DeviceId,
    context: ConnectionContext,
    motion: RelativeMotionDatagram,
}
#[derive(Default)]
pub(super) struct MotionInbox {
    pending: Mutex<BTreeMap<(TransportSessionId, InputSessionId), ReceivedMotion>>,
    relative: Mutex<BTreeMap<(TransportSessionId, InputSessionId), ReceivedRelative>>,
    ready: Notify,
}
impl MotionInbox {
    /// Returns whether an old sample was replaced, or None for a drop.
    fn enqueue(&self, session: &Session, motion: MotionDatagram) -> Option<bool> {
        let mut pending = self.pending.lock().ok()?;
        let key = (session.context.session_id, motion.session_id);
        let replaced = if let Some(previous) = pending.get(&key) {
            if previous.motion.token == motion.token
                && (previous.motion.position.motion_seq >= motion.position.motion_seq
                    || previous.motion.state_epoch > motion.state_epoch)
            {
                return None;
            }
            true
        } else {
            if pending.len() >= MAX_RECEIVE_SLOTS
                || pending.keys().filter(|(id, _)| *id == key.0).count() >= MAX_CONTROL_SLOTS
            {
                return None;
            }
            false
        };
        pending.insert(
            key,
            ReceivedMotion {
                peer: session.peer,
                context: session.context.clone(),
                motion,
            },
        );
        drop(pending);
        self.ready.notify_one();
        Some(replaced)
    }

    /// 接收侧同样累加。只留最新相对包会丢掉已经在途的位移。
    fn enqueue_relative(&self, session: &Session, motion: RelativeMotionDatagram) -> Option<bool> {
        let mut pending = self.relative.lock().ok()?;
        let key = (session.context.session_id, motion.session_id);
        // 先拷出要比较的字段，再改槽，避免借用还活着时 remove。
        let previous = pending.get(&key).map(|item| {
            (
                item.motion.token,
                item.motion.state_epoch,
                item.motion.motion_seq,
                item.motion.dx,
                item.motion.dy,
            )
        });
        let replaced = if let Some((token, epoch, seq, prev_dx, prev_dy)) = previous {
            if token == motion.token && epoch == motion.state_epoch {
                if motion.motion_seq <= seq {
                    return None;
                }
                let dx = prev_dx + motion.dx;
                let dy = prev_dy + motion.dy;
                if !relative_delta_ok(dx) || !relative_delta_ok(dy) {
                    pending.remove(&key);
                    self.ready.notify_one();
                    return Some(true);
                }
                let mut combined = motion;
                combined.dx = dx;
                combined.dy = dy;
                pending.insert(
                    key,
                    ReceivedRelative {
                        peer: session.peer,
                        context: session.context.clone(),
                        motion: combined,
                    },
                );
                drop(pending);
                self.ready.notify_one();
                return Some(false);
            }
            if token == motion.token && epoch > motion.state_epoch {
                return None;
            }
            true
        } else {
            if pending.len() >= MAX_RECEIVE_SLOTS
                || pending.keys().filter(|(id, _)| *id == key.0).count() >= MAX_CONTROL_SLOTS
            {
                return None;
            }
            false
        };
        pending.insert(
            key,
            ReceivedRelative {
                peer: session.peer,
                context: session.context.clone(),
                motion,
            },
        );
        drop(pending);
        self.ready.notify_one();
        Some(replaced)
    }

    pub(super) async fn recv(&self) -> InputTransportEvent {
        loop {
            if let Some(received) = self.pending.lock().ok().and_then(|mut p| p.pop_first()) {
                return InputTransportEvent::Motion {
                    peer_id: received.1.peer,
                    context: received.1.context,
                    motion: received.1.motion,
                };
            }
            if let Some(received) = self.relative.lock().ok().and_then(|mut p| p.pop_first()) {
                return InputTransportEvent::RelativeMotion {
                    peer_id: received.1.peer,
                    context: received.1.context,
                    motion: received.1.motion,
                };
            }
            self.ready.notified().await;
        }
    }
    pub(super) fn remove_transport(&self, transport: TransportSessionId) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.retain(|(id, _), _| *id != transport);
        }
        if let Ok(mut pending) = self.relative.lock() {
            pending.retain(|(id, _), _| *id != transport);
        }
    }
    fn remove(&self, transport: TransportSessionId, control: InputSessionId) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&(transport, control));
        }
        if let Ok(mut pending) = self.relative.lock() {
            pending.remove(&(transport, control));
        }
    }
}

fn datagram_supported(session: &Session) -> bool {
    session
        .negotiated
        .contains(Capabilities::INPUT_MOTION_DATAGRAM_V1)
        && session
            .connection
            .max_datagram_size()
            .is_some_and(|max| max >= MAX_MOTION_DATAGRAM_SIZE)
}
fn probe(kind: u8, nonce: &[u8; 32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(PROBE_SIZE);
    bytes.extend_from_slice(PROBE_MAGIC);
    bytes.push(kind);
    bytes.extend_from_slice(nonce);
    bytes
}
fn decode_probe(bytes: &[u8]) -> Option<(u8, [u8; 32])> {
    if bytes.len() != PROBE_SIZE
        || !bytes.starts_with(PROBE_MAGIC)
        || !matches!(bytes[8], PROBE_PING | PROBE_PONG)
    {
        return None;
    }
    Some((bytes[8], bytes[9..].try_into().ok()?))
}

impl InputHub {
    pub fn motion_available(&self, peer: DeviceId, id: TransportSessionId) -> bool {
        self.sessions
            .lock()
            .ok()
            .and_then(|sessions| {
                sessions.get(&id).map(|session| {
                    session.peer == peer
                        && session.live()
                        && session.context.input_eligible()
                        && datagram_supported(session)
                        && session.motion.ready.load(Ordering::Acquire)
                })
            })
            .unwrap_or(false)
    }

    pub fn motion_diagnostics(
        &self,
        peer: DeviceId,
        id: TransportSessionId,
    ) -> InputMotionDiagnostics {
        let Some(session) = self
            .sessions
            .lock()
            .ok()
            .and_then(|sessions| sessions.get(&id).filter(|s| s.peer == peer).cloned())
        else {
            return InputMotionDiagnostics::default();
        };
        let link = &session.motion;
        let path = session.connection.stats().path;
        InputMotionDiagnostics {
            available: self.motion_available(peer, id),
            sent: link.sent.load(Ordering::Relaxed),
            received: link.received.load(Ordering::Relaxed),
            send_replaced: link.send_replaced.load(Ordering::Relaxed),
            receive_replaced: link.receive_replaced.load(Ordering::Relaxed),
            dropped: link.dropped.load(Ordering::Relaxed),
            pending_send: link.pending.lock().map(|p| p.len()).unwrap_or_default()
                + link
                    .pending_relative
                    .lock()
                    .map(|p| p.len())
                    .unwrap_or_default(),
            rtt_micros: path.rtt.as_micros().min(u128::from(u64::MAX)) as u64,
            lost_packets: path.lost_packets,
            sent_packets: path.sent_packets,
            congestion_events: path.congestion_events,
        }
    }

    pub fn send_motion(
        &self,
        peer: DeviceId,
        id: TransportSessionId,
        motion: MotionDatagram,
    ) -> Result<(), TransportError> {
        motion
            .validate()
            .map_err(|_| TransportError::MalformedPeer("invalid motion datagram"))?;
        // Encode here as well as at flush so callers synchronously see wire-size errors.
        let bytes = encode_motion_datagram(&motion).map_err(TransportError::Protocol)?;
        let session = self
            .sessions
            .lock()
            .map_err(|_| TransportError::Closed)?
            .get(&id)
            .filter(|s| s.peer == peer && s.live() && s.context.input_eligible())
            .cloned()
            .ok_or(TransportError::Closed)?;
        if !datagram_supported(&session)
            || !session.motion.ready.load(Ordering::Acquire)
            || session
                .connection
                .max_datagram_size()
                .is_none_or(|max| bytes.len() > max)
        {
            return Err(TransportError::Closed);
        }
        session.motion.enqueue(motion)
    }

    pub fn send_relative(
        &self,
        peer: DeviceId,
        id: TransportSessionId,
        motion: RelativeMotionDatagram,
    ) -> Result<(), TransportError> {
        motion
            .validate()
            .map_err(|_| TransportError::MalformedPeer("invalid relative motion datagram"))?;
        let bytes = encode_relative_motion_datagram(&motion).map_err(TransportError::Protocol)?;
        let session = self
            .sessions
            .lock()
            .map_err(|_| TransportError::Closed)?
            .get(&id)
            .filter(|s| s.peer == peer && s.live() && s.context.input_eligible())
            .cloned()
            .ok_or(TransportError::Closed)?;
        if !datagram_supported(&session)
            || !session.motion.ready.load(Ordering::Acquire)
            || session
                .connection
                .max_datagram_size()
                .is_none_or(|max| bytes.len() > max)
        {
            return Err(TransportError::Closed);
        }
        session.motion.enqueue_relative(motion)
    }

    pub fn cancel_motion(&self, peer: DeviceId, id: TransportSessionId, control: InputSessionId) {
        if let Some(session) = self
            .sessions
            .lock()
            .ok()
            .and_then(|s| s.get(&id).filter(|s| s.peer == peer).cloned())
        {
            // Same lock as the synchronous flush: no in-flight application slot
            // can be reinserted after cancellation. QUIC-queued/late network data
            // still requires runtime epoch/session validation.
            if let Ok(mut pending) = session.motion.pending.lock() {
                pending.remove(&control);
            }
            if let Ok(mut pending) = session.motion.pending_relative.lock() {
                pending.remove(&control);
            }
            self.motion_inbox.remove(id, control);
        }
    }

    pub(super) fn start_motion(&self, session: Session) {
        let Some(executor) = &self.executor else {
            return;
        };
        let Ok(sessions) = self.sessions.lock() else {
            return;
        };
        if sessions.values().any(|other| {
            other.context.session_id != session.context.session_id
                && other.connection.stable_id() == session.connection.stable_id()
                && other.motion.sampling_started.load(Ordering::Acquire)
        }) || session.motion.sampling_started.swap(true, Ordering::AcqRel)
        {
            return;
        }
        drop(sessions);
        let hub = self.clone();
        // 独立采样器：质量曲线只描述链路本身，不能挂在 `ready` 或探针发送成功上。
        // ready 掉了之后运动改走可靠通道、drain 队列也随之空掉，原先那条
        // 「ready 才采样」的路径会让曲线毫无提示地冻在降级前的位置。
        let sampling = session.clone();
        executor.spawn(async move {
            while sampling.live() {
                tokio::select! {
                    _ = tokio::time::sleep(SAMPLE_INTERVAL) => {}
                    _ = sampling.cancel.notified() => break,
                }
                sampling
                    .motion
                    .sample_diagnostics(&sampling, "sample", false);
            }
        });
        // Reliable-only connections still have a live QUIC path and RTT.
        if !datagram_supported(&session) {
            return;
        }
        session.motion.started.store(true, Ordering::Release);
        let reader = session.clone();
        executor.spawn(async move {
            let mut last_ping_response: Option<tokio::time::Instant> = None;
            while reader.live() {
                let received = tokio::select! {
                    bytes = reader.connection.read_datagram() => bytes,
                    _ = reader.cancel.notified() => break,
                };
                let Ok(bytes) = received else {
                    break;
                };
                if !reader.live() {
                    break;
                }
                if let Some((kind, incoming_nonce)) = decode_probe(&bytes) {
                    if kind == PROBE_PING {
                        // A peer cannot turn probe reflection into an unbounded
                        // source of queued datagrams which displaces real motion.
                        if last_ping_response
                            .is_none_or(|at| at.elapsed() >= Duration::from_millis(100))
                        {
                            let _ = reader
                                .connection
                                .send_datagram(probe(PROBE_PONG, &incoming_nonce).into());
                            last_ping_response = Some(tokio::time::Instant::now());
                        }
                    } else if datagram_supported(&reader)
                        && reader
                            .motion
                            .probe
                            .lock()
                            .is_ok_and(|mut state| state.acknowledge(incoming_nonce))
                    {
                        // Our challenge's echo proves both directions, not just
                        // the transport parameter advertised by the peer.
                        if !reader.motion.ready.swap(true, Ordering::AcqRel) {
                            reader.motion.sample_diagnostics(&reader, "ready", true);
                        }
                    }
                    continue;
                }
                if bytes.len() > MAX_MOTION_DATAGRAM_SIZE
                    || !reader.motion.ready.load(Ordering::Acquire)
                {
                    reader.motion.dropped.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                if bytes.len() >= 4 && bytes[..4] == [b'D', b'F', b'R', 1] {
                    let Ok(motion) = decode_relative_motion_datagram(&bytes) else {
                        reader.motion.dropped.fetch_add(1, Ordering::Relaxed);
                        continue;
                    };
                    reader.motion.received.fetch_add(1, Ordering::Relaxed);
                    match hub.motion_inbox.enqueue_relative(&reader, motion) {
                        Some(true) => {
                            reader
                                .motion
                                .receive_replaced
                                .fetch_add(1, Ordering::Relaxed);
                        }
                        Some(false) => {}
                        None => {
                            reader.motion.dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    reader.motion.sample_diagnostics(&reader, "receive", false);
                    continue;
                }
                let Ok(motion) = decode_motion_datagram(&bytes) else {
                    reader.motion.dropped.fetch_add(1, Ordering::Relaxed);
                    continue;
                };
                reader.motion.received.fetch_add(1, Ordering::Relaxed);
                match hub.motion_inbox.enqueue(&reader, motion) {
                    Some(true) => {
                        reader
                            .motion
                            .receive_replaced
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    Some(false) => {}
                    None => {
                        reader.motion.dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
                reader.motion.sample_diagnostics(&reader, "receive", false);
            }
            reader.motion.ready.store(false, Ordering::Release);
        });
        let probing = session.clone();
        tokio::spawn(async move {
            while probing.live() {
                let nonce = if let Ok(mut state) = probing.motion.probe.lock() {
                    if !state.healthy() {
                        probing.motion.ready.store(false, Ordering::Release);
                    }
                    state.challenge()
                } else {
                    break;
                };
                if probing
                    .connection
                    .send_datagram(probe(PROBE_PING, &nonce).into())
                    .is_err()
                {
                    probing.motion.close();
                    break;
                }
                // 采样由 start_motion 里的独立采样器负责，与这里的探针成败无关。
                tokio::select! {
                    _ = tokio::time::sleep(PROBE_INTERVAL) => {},
                    _ = probing.cancel.notified() => break,
                }
            }
        });
        executor.spawn(async move {
            // Match Deskflow's motion path: writes are event-driven and immediate.
            // The mailbox already keeps only the newest position per input session;
            // draining it here handles every slot covered by one Notify wake without
            // imposing an application-level frame rate.  The receiver performs the
            // final adjacent-motion coalescing when data is already queued.
            let mut drain_count = 0u64;
            let mut wake_count = 0u64;
            while session.live() {
                tokio::select! {
                    _ = session.motion.wake.notified() => {},
                    _ = session.cancel.notified() => break,
                    // Bound shutdown even if Notify cancellation races registration.
                    _ = tokio::time::sleep(Duration::from_secs(1)) => continue,
                }
                if !session.live() {
                    break;
                }
                wake_count += 1;
                let drain_started = Instant::now();
                let mut batch_sent = 0usize;

                // Double-check drain loop: prevents Notify merge from losing samples.
                // After exhausting the mailbox, check once more that nothing arrived
                // during the final pop_first(), because concurrent enqueue + notify
                // between our empty check and the outer notified().await would merge
                // into one permit and skip a wake cycle.
                'drain: loop {
                    if !session.live() {
                        break 'drain;
                    }

                    let next = session.motion.pop_outgoing();

                    let Some(next) = next else {
                        // Final check: if mailbox became non-empty between our last
                        // pop and here, drain again instead of sleeping.
                        if session.motion.has_outgoing() {
                            continue 'drain;
                        }
                        break 'drain;
                    };

                    let encode_start = Instant::now();
                    let encoded = match &next {
                        OutgoingMotion::Absolute(motion) => encode_motion_datagram(motion).ok(),
                        OutgoingMotion::Relative(motion) => {
                            encode_relative_motion_datagram(motion).ok()
                        }
                    };
                    let encode_time = encode_start.elapsed();

                    let send_start = Instant::now();
                    let sent = session.motion.ready.load(Ordering::Acquire)
                        && encoded.is_some_and(|bytes| {
                            session.connection.send_datagram(bytes.into()).is_ok()
                        });
                    let send_time = send_start.elapsed();

                    let health_lost = if sent {
                        session.motion.sent.fetch_add(1, Ordering::Relaxed);
                        record_input_stage(
                            InputStage::Send,
                            u64::try_from(send_time.as_micros()).unwrap_or(u64::MAX),
                        );
                        batch_sent += 1;
                        false
                    } else {
                        session.motion.dropped.fetch_add(1, Ordering::Relaxed);
                        session.motion.ready.store(false, Ordering::Release);
                        true
                    };

                    if deskfuse_core::debug_logging::enabled()
                        && (health_lost
                            || encode_time >= Duration::from_micros(500)
                            || send_time >= Duration::from_micros(500))
                    {
                        crate::motion_diag::record(crate::motion_diag::MotionDiagRecord {
                            timestamp_ms: crate::motion_diag::timestamp_ms(),
                            event: if health_lost {
                                "send_health_lost"
                            } else {
                                "send_slow"
                            },
                            peer: crate::motion_diag::short_peer(session.peer),
                            transport_session: crate::motion_diag::short_session(
                                session.context.session_id,
                            ),
                            local_addr: session.context.local_addr.to_string(),
                            remote_addr: session.context.remote_addr.to_string(),
                            interface_name: session.context.interface_name.clone(),
                            interface_kind: format!("{:?}", session.context.interface_kind),
                            ready: session.motion.ready.load(Ordering::Acquire),
                            sent: session.motion.sent.load(Ordering::Relaxed),
                            received: session.motion.received.load(Ordering::Relaxed),
                            send_replaced: session.motion.send_replaced.load(Ordering::Relaxed),
                            receive_replaced: session
                                .motion
                                .receive_replaced
                                .load(Ordering::Relaxed),
                            dropped: session.motion.dropped.load(Ordering::Relaxed),
                            pending_send: 0,
                            rtt_micros: 0,
                            sent_packets: 0,
                            lost_packets: 0,
                            congestion_events: 0,
                        });
                    }

                    session.motion.sample_diagnostics(
                        &session,
                        if health_lost { "health_lost" } else { "send" },
                        health_lost,
                    );

                    // Give cancellation and the QUIC driver a scheduling point, but
                    // never sleep to manufacture a motion frame rate.
                    tokio::task::yield_now().await;

                    if health_lost {
                        break 'drain;
                    }
                }

                drain_count += 1;
                let drain_time = drain_started.elapsed();

                // Log slow or large drain cycles to correlate sender pacing with
                // receiver gaps. A long drain or high batch count may indicate that
                // motion enqueue is outpacing the async executor's yield scheduling.
                if deskfuse_core::debug_logging::enabled()
                    && (drain_time >= Duration::from_millis(8) || batch_sent >= 16)
                {
                    crate::motion_diag::record(crate::motion_diag::MotionDiagRecord {
                        timestamp_ms: crate::motion_diag::timestamp_ms(),
                        event: "drain_cycle",
                        peer: crate::motion_diag::short_peer(session.peer),
                        transport_session: crate::motion_diag::short_session(
                            session.context.session_id,
                        ),
                        local_addr: session.context.local_addr.to_string(),
                        remote_addr: session.context.remote_addr.to_string(),
                        interface_name: session.context.interface_name.clone(),
                        interface_kind: format!("{:?}", session.context.interface_kind),
                        ready: session.motion.ready.load(Ordering::Acquire),
                        sent: batch_sent as u64,
                        received: 0,
                        send_replaced: 0,
                        receive_replaced: 0,
                        dropped: 0,
                        pending_send: session
                            .motion
                            .pending
                            .lock()
                            .ok()
                            .map(|p| p.len())
                            .unwrap_or(0),
                        rtt_micros: drain_time.as_micros().min(u128::from(u64::MAX)) as u64,
                        sent_packets: wake_count,
                        lost_packets: drain_count,
                        congestion_events: 0,
                    });
                }
            }
            session.motion.close();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deskfuse_core::{DisplayId, InputShareMessage, InterfaceKind, LinkScope, MotionPosition};
    use futures::StreamExt;
    use std::{net::SocketAddr, sync::Arc};

    fn session(
        connection: quinn::Connection,
        local: SocketAddr,
        negotiated: Capabilities,
    ) -> Session {
        Session {
            negotiated,
            peer: DeviceId::from_bytes([41; 32]),
            context: ConnectionContext {
                session_id: TransportSessionId::new(),
                local_addr: local,
                remote_addr: connection.remote_address(),
                interface_name: None,
                interface_kind: InterfaceKind::Loopback,
                scope: LinkScope::Unknown,
            },
            connection,
            valid: Arc::new(AtomicBool::new(true)),
            cancel: Arc::new(crate::input::CancelSignal::default()),
            motion: Arc::default(),
        }
    }
    fn sample(id: InputSessionId, seq: u64) -> MotionDatagram {
        MotionDatagram {
            session_id: id,
            token: InputToken([8; 32]),
            state_epoch: 1,
            position: MotionPosition {
                target_display: DisplayId("display".into()),
                topology_generation: 1,
                motion_seq: seq,
                x: 10.0,
                y: 20.0,
            },
        }
    }
    async fn ready(a: &Session, b: &Session) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !a.motion.ready.load(Ordering::Acquire) || !b.motion.ready.load(Ordering::Acquire)
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("loopback probes must complete in both directions");
    }

    #[test]
    fn probe_parser_has_fixed_bounds_and_disjoint_motion_prefix() {
        let nonce = [5; 32];
        let ping = probe(PROBE_PING, &nonce);
        assert_eq!(decode_probe(&ping), Some((PROBE_PING, nonce)));
        for length in 0..PROBE_SIZE {
            assert_eq!(decode_probe(&ping[..length]), None);
        }
        let mut oversized = ping.clone();
        oversized.push(0);
        assert_eq!(decode_probe(&oversized), None);
        assert_eq!(decode_probe(&probe(3, &nonce)), None);
        let encoded = encode_motion_datagram(&sample(InputSessionId::new(), 1)).unwrap();
        assert_eq!(decode_probe(&encoded), None);
        assert!(decode_motion_datagram(&ping).is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn health_probe_rejects_wrong_replayed_and_expired_pongs() {
        let mut state = ProbeState::default();
        assert!(!state.healthy());
        let nonce = state.challenge();
        assert!(!state.acknowledge([0; 32]));
        assert!(state.acknowledge(nonce));
        assert!(state.healthy());
        assert!(!state.acknowledge(nonce));
        let next = state.challenge();
        assert_ne!(nonce, next);
        assert!(!state.acknowledge(nonce));
        tokio::time::advance(PROBE_TIMEOUT).await;
        assert!(!state.healthy());
        assert!(!state.acknowledge(next));
        let fresh = state.challenge();
        assert_ne!(next, fresh);
        assert!(state.acknowledge(fresh));
        assert!(state.healthy());
    }

    #[test]
    fn send_slots_bound_bursts_and_drop_queued_reordering() {
        let link = MotionLink::default();
        let id = InputSessionId::new();
        for seq in 1..=10_000 {
            link.enqueue(sample(id, seq)).unwrap();
        }
        link.enqueue(sample(id, 9_999)).unwrap();
        link.enqueue(sample(id, 10_000)).unwrap();
        assert_eq!(link.pending.lock().unwrap().len(), 1);
        assert_eq!(
            link.pending.lock().unwrap()[&id].position.motion_seq,
            10_000
        );
        assert_eq!(link.send_replaced.load(Ordering::Relaxed), 9_999);
        assert_eq!(link.dropped.load(Ordering::Relaxed), 2);
        for _ in 1..MAX_CONTROL_SLOTS {
            link.enqueue(sample(InputSessionId::new(), 1)).unwrap();
        }
        assert!(link.enqueue(sample(InputSessionId::new(), 1)).is_err());
        // Replacing an existing slot still succeeds at capacity.
        link.enqueue(sample(id, 10_001)).unwrap();
        assert_eq!(link.pending.lock().unwrap().len(), MAX_CONTROL_SLOTS);
        link.close();
        assert!(link.pending.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn loopback_probes_and_motion_use_real_datagrams_not_reliable_channel() {
        let (server, client, a, b) = super::super::tests::connections().await;
        let caps = Capabilities::INPUT_SHARING | Capabilities::INPUT_MOTION_DATAGRAM_V1;
        let a = session(a, client.local_addr().unwrap(), caps);
        let b = session(b, server.local_addr().unwrap(), caps);
        let sender = InputHub::new();
        let receiver = InputHub::new();
        assert!(sender.admit_session(a.clone()));
        assert!(receiver.admit_session(b.clone()));
        sender.start_motion(a.clone());
        receiver.start_motion(b.clone());
        ready(&a, &b).await;
        // Socket policy is not bypassed: a loopback context remains ineligible
        // through the public API. Exercise the exact worker below that boundary.
        assert!(!sender.motion_available(a.peer, a.context.session_id));
        let id = InputSessionId::new();
        assert!(
            sender
                .send_motion(a.peer, a.context.session_id, sample(id, 1))
                .is_err()
        );
        // Coalescing is covered by send_slots_bound_bursts_and_drop_queued_reordering.
        // The real worker runs on a separate executor and may drain during a
        // producer burst; that burst cannot deterministically mean one packet.
        a.motion.enqueue(sample(id, 10_000)).unwrap();
        let mut stream = receiver.stream();
        let event = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap();
        let InputTransportEvent::Motion {
            context, motion, ..
        } = event
        else {
            panic!("not motion");
        };
        assert_eq!(context.session_id, b.context.session_id);
        assert_eq!(motion.position.motion_seq, 10_000);
        assert_eq!(a.motion.sent.load(Ordering::Relaxed), 1);
        assert_eq!(b.motion.received.load(Ordering::Relaxed), 1);
        assert_eq!(receiver.tx.capacity(), 512);
        sender.invalidate_peer(a.peer);
        receiver.invalidate_peer(b.peer);
        assert!(!a.motion.ready.load(Ordering::Acquire));
        assert!(sender.sessions.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn old_peer_is_never_probed_and_cannot_enable_motion() {
        let (_server, client, a, b) = super::super::tests::connections().await;
        let hub = InputHub::new();
        let a = session(a, client.local_addr().unwrap(), Capabilities::INPUT_SHARING);
        assert!(hub.admit_session(a.clone()));
        hub.start_motion(a.clone());
        assert!(!a.motion.started.load(Ordering::Acquire));
        assert!(!a.motion.ready.load(Ordering::Acquire));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), b.read_datagram())
                .await
                .is_err()
        );
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        assert!(
            a.motion.diag_at.lock().unwrap().is_some(),
            "reliable-only links must still sample quality"
        );
        assert!(!a.motion.ready.load(Ordering::Acquire));
        hub.invalidate_peer(a.peer);
        assert!(super::super::requires_motion_capability(
            &InputShareMessage::RequestHybrid {
                session_id: InputSessionId::new(),
                target_display: DisplayId("display".into()),
                relative_ready: false,
            }
        ));
    }

    #[tokio::test]
    async fn ready_link_keeps_sampling_quality_without_motion() {
        let (server, client, a, b) = super::super::tests::connections().await;
        let caps = Capabilities::INPUT_SHARING | Capabilities::INPUT_MOTION_DATAGRAM_V1;
        let a = session(a, client.local_addr().unwrap(), caps);
        let b = session(b, server.local_addr().unwrap(), caps);
        let sender = InputHub::new();
        let receiver = InputHub::new();
        assert!(sender.admit_session(a.clone()));
        assert!(receiver.admit_session(b.clone()));
        sender.start_motion(a.clone());
        receiver.start_motion(b.clone());
        ready(&a, &b).await;
        // No motion is sent at all: the probe loop alone must keep the
        // once-per-second quality records flowing so the chart stays continuous.
        let baseline = crate::motion_diag::quality_snapshot().len();
        tokio::time::sleep(Duration::from_millis(2_300)).await;
        let sampled = crate::motion_diag::quality_snapshot().len();
        assert!(
            sampled >= baseline + 2,
            "ready links must keep sampling without motion: {baseline} -> {sampled}"
        );
        sender.invalidate_peer(a.peer);
        receiver.invalidate_peer(b.peer);
    }

    #[tokio::test]
    async fn generic_quic_connections_do_not_negotiate_datagrams() {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let der = cert.cert.der().to_vec();
        let local = "127.0.0.1:0".parse().unwrap();
        let config = crate::QuinnTransportConfig::new(
            local,
            local,
            "localhost",
            der.clone(),
            cert.key_pair.serialize_der(),
            der,
        );
        let (server_config, client_config) = crate::build_quic_configs(&config).unwrap();
        let server = quinn::Endpoint::server(server_config, local).unwrap();
        let mut client = quinn::Endpoint::client(local).unwrap();
        client.set_default_client_config(client_config);
        let outgoing = client
            .connect(server.local_addr().unwrap(), "localhost")
            .unwrap();
        let (a, b) = tokio::join!(outgoing, async {
            server.accept().await.unwrap().await.unwrap()
        });
        let a = a.unwrap();
        assert!(a.max_datagram_size().is_none());
        assert!(b.max_datagram_size().is_none());
        assert!(matches!(
            a.send_datagram(vec![1].into()),
            Err(quinn::SendDatagramError::Disabled)
        ));
    }

    #[tokio::test]
    async fn motion_flood_cannot_consume_reliable_capacity_or_reorder_barriers() {
        let (_server, client, a, _b) = super::super::tests::connections().await;
        let session = session(a, client.local_addr().unwrap(), Capabilities::INPUT_SHARING);
        let hub = InputHub::new();
        let id = InputSessionId::new();
        for seq in 1..=10_000 {
            assert!(
                hub.motion_inbox
                    .enqueue(&session, sample(id, seq))
                    .is_some()
            );
        }
        assert_eq!(hub.motion_inbox.enqueue(&session, sample(id, 9999)), None);
        for sequence in 1..=512 {
            hub.tx
                .try_send(InputTransportEvent::Message {
                    peer_id: session.peer,
                    context: session.context.clone(),
                    message: InputShareMessage::Renew {
                        session_id: id,
                        token: InputToken([8; 32]),
                        sequence,
                    },
                })
                .unwrap();
        }
        assert_eq!(hub.motion_inbox.pending.lock().unwrap().len(), 1);
        let mut stream = hub.stream();
        for expected in 1..=512 {
            let Some(InputTransportEvent::Message {
                message: InputShareMessage::Renew { sequence, .. },
                ..
            }) = stream.next().await
            else {
                panic!("motion displaced reliable state");
            };
            assert_eq!(sequence, expected);
        }
        let Some(InputTransportEvent::Motion { motion, .. }) = stream.next().await else {
            panic!("missing motion");
        };
        assert_eq!(motion.position.motion_seq, 10_000);
    }

    #[tokio::test]
    async fn cancel_clears_both_directions_without_deleting_other_control_slots() {
        let (_server, client, a, _b) = super::super::tests::connections().await;
        let session = session(a, client.local_addr().unwrap(), Capabilities::INPUT_SHARING);
        let hub = InputHub::new();
        assert!(hub.admit_session(session.clone()));
        let id = InputSessionId::new();
        let other = InputSessionId::new();
        for control in [id, other] {
            session.motion.enqueue(sample(control, 1)).unwrap();
            hub.motion_inbox
                .enqueue(&session, sample(control, 1))
                .unwrap();
        }
        hub.cancel_motion(
            DeviceId::from_bytes([0; 32]),
            session.context.session_id,
            id,
        );
        assert_eq!(session.motion.pending.lock().unwrap().len(), 2);
        hub.cancel_motion(session.peer, session.context.session_id, id);
        assert_eq!(session.motion.pending.lock().unwrap().len(), 1);
        assert!(!session.motion.pending.lock().unwrap().contains_key(&id));
        assert_eq!(hub.motion_inbox.pending.lock().unwrap().len(), 1);
        // Leave does not permanently ban a lease ID; a later reliable Enter
        // establishes the new epoch at the runtime layer.
        session.motion.enqueue(sample(id, 2)).unwrap();
        hub.invalidate_peer(session.peer);
        assert!(session.motion.pending.lock().unwrap().is_empty());
        assert!(hub.motion_inbox.pending.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn regressing_epoch_cannot_replace_a_newer_pending_state() {
        let (_server, client, a, _b) = super::super::tests::connections().await;
        let session = session(a, client.local_addr().unwrap(), Capabilities::INPUT_SHARING);
        let inbox = MotionInbox::default();
        let id = InputSessionId::new();
        let mut current = sample(id, 2);
        current.state_epoch = 2;
        session.motion.enqueue(current.clone()).unwrap();
        assert_eq!(inbox.enqueue(&session, current), Some(false));
        let stale = sample(id, 3);
        session.motion.enqueue(stale.clone()).unwrap();
        assert_eq!(inbox.enqueue(&session, stale), None);
        assert_eq!(session.motion.pending.lock().unwrap()[&id].state_epoch, 2);
        assert_eq!(
            inbox.pending.lock().unwrap()[&(session.context.session_id, id)]
                .motion
                .state_epoch,
            2
        );
    }

    #[tokio::test]
    async fn receive_slots_have_per_connection_and_global_bounds() {
        let (_server, client, a, _b) = super::super::tests::connections().await;
        let session = session(a, client.local_addr().unwrap(), Capabilities::INPUT_SHARING);
        let inbox = MotionInbox::default();
        for _ in 0..MAX_CONTROL_SLOTS {
            inbox
                .enqueue(&session, sample(InputSessionId::new(), 1))
                .unwrap();
        }
        assert_eq!(
            inbox.enqueue(&session, sample(InputSessionId::new(), 1)),
            None
        );
        for _ in MAX_CONTROL_SLOTS..MAX_RECEIVE_SLOTS {
            let mut another = session.clone();
            another.context.session_id = TransportSessionId::new();
            inbox
                .enqueue(&another, sample(InputSessionId::new(), 1))
                .unwrap();
        }
        let mut another = session.clone();
        another.context.session_id = TransportSessionId::new();
        assert_eq!(
            inbox.enqueue(&another, sample(InputSessionId::new(), 1)),
            None
        );
        assert_eq!(inbox.pending.lock().unwrap().len(), MAX_RECEIVE_SLOTS);
        inbox.remove_transport(session.context.session_id);
        assert_eq!(
            inbox.pending.lock().unwrap().len(),
            MAX_RECEIVE_SLOTS - MAX_CONTROL_SLOTS
        );
    }

    #[tokio::test]
    async fn raw_reordered_datagrams_deliver_only_latest_pending_position() {
        let (server, _client, a, b) = super::super::tests::connections().await;
        let caps = Capabilities::INPUT_SHARING | Capabilities::INPUT_MOTION_DATAGRAM_V1;
        let receiving = session(b, server.local_addr().unwrap(), caps);
        let hub = InputHub::new();
        assert!(hub.admit_session(receiving.clone()));
        hub.start_motion(receiving.clone());
        // This endpoint deliberately behaves like a protocol peer to exercise
        // the real reader with malformed and reordered wire frames.
        let ping = tokio::time::timeout(Duration::from_secs(1), a.read_datagram())
            .await
            .unwrap()
            .unwrap();
        let (kind, nonce) = decode_probe(&ping).unwrap();
        assert_eq!(kind, PROBE_PING);
        a.send_datagram(probe(PROBE_PONG, &nonce).into()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !receiving.motion.ready.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let id = InputSessionId::new();
        for seq in [3, 1, 2] {
            a.send_datagram(encode_motion_datagram(&sample(id, seq)).unwrap().into())
                .unwrap();
        }
        a.send_datagram(vec![0xff; MAX_MOTION_DATAGRAM_SIZE].into())
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while receiving.motion.received.load(Ordering::Relaxed) < 3
                || receiving.motion.dropped.load(Ordering::Relaxed) < 3
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let InputTransportEvent::Motion { motion, .. } = hub.motion_inbox.recv().await else {
            unreachable!()
        };
        assert_eq!(motion.position.motion_seq, 3);
        assert!(receiving.live()); // Expected loss/parse drops never revoke input.
        // Stop responding while the QUIC connection remains alive. Health must
        // become false even though max_datagram_size is still negotiated.
        tokio::time::timeout(Duration::from_secs(3), async {
            while receiving.motion.ready.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        assert!(receiving.live());
        hub.invalidate_peer(receiving.peer);
    }
}
