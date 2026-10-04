//! Runs inside the existing signed input service. Cross-screen capture/injection
//! must acknowledge a pause before this independent lease may arm the adapter.
use deskfuse_core::*;
use deskfuse_platform::SharedInputInjector;
use deskfuse_platform::ports::EnginePlatformPort;
use deskfuse_runtime::{
    input_service::Authority, input_service_types::RemoteApplicationInput as Packet,
};
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};
const TTL: Duration = Duration::from_secs(1);
pub struct Owner {
    pause: Arc<AtomicU8>,
    platform: Arc<dyn EnginePlatformPort>,
    pending: Option<(Packet, Instant)>,
    buffered: VecDeque<Packet>,
    active: Option<(
        Packet,
        InputSessionId,
        DisplayDescriptor,
        Box<dyn SharedInputInjector>,
    )>,
    deadline: Instant,
    sequence: u64,
    last_log: Instant,
    received: u64,
    injected: u64,
    pub request: Option<String>,
    pub quiesced: Option<String>,
    pub error: Option<String>,
    release_failed: bool,
}
impl Owner {
    pub fn new(pause: Arc<AtomicU8>, platform: Arc<dyn EnginePlatformPort>) -> Self {
        Self {
            pause,
            platform,
            pending: None,
            buffered: VecDeque::new(),
            active: None,
            deadline: Instant::now(),
            sequence: 0,
            last_log: Instant::now(),
            received: 0,
            injected: 0,
            request: None,
            quiesced: None,
            error: None,
            release_failed: false,
        }
    }
    fn allowed(packet: &Packet, authority: &Authority) -> bool {
        packet.peer.is_some_and(|id| {
            authority.network.allows(authority.local, id)
                && !authority.network.blocked_peer_ids.contains(&id)
                && authority
                    .peers
                    .iter()
                    .any(|p| p.id == id && p.trust == TrustState::Trusted)
        })
    }
    fn same(a: &Packet, b: &Packet) -> bool {
        a.peer == b.peer && a.session == b.session && a.generation == b.generation
    }
    fn diagnostic(&mut self, reason: &str) {
        let problem = match reason {
            "exclusive_ownership_lost" => Some("输入服务的独占控制已被收回"),
            "heartbeat_timeout" | "native_lease_expired" => Some("远程输入心跳超时，已释放控制"),
            "authority_revoked" | "packet_unauthorized" => Some("远程应用访问授权已失效"),
            "system_session_inactive" => Some("被控 Mac 已锁屏或用户会话不活跃"),
            "foreground_changed" => Some("被控 Mac 的前台应用已切换，点击画面重新获取控制"),
            "native_injection_error" | "enqueue_failed" => {
                Some("被控 Mac 的原生输入注入失败，已释放控制")
            }
            "configure_timeout_or_revoked" => Some("被控 Mac 未及时释放跨屏键鼠占用，控制申请超时"),
            "configure_pause_busy" => Some("被控 Mac 正在更新或控制其他设备，无法取得输入独占权"),
            "permission_denied" => {
                Some("被控 Mac 的输入服务没有辅助功能权限，请在系统设置中允许 DeskFuse")
            }
            "native_arm_failed" => Some("被控 Mac 的原生输入服务启动失败"),
            "emergency" => Some("被控 Mac 已紧急收回控制"),
            _ => None,
        };
        if let Some(problem) = problem {
            self.error = Some(problem.into());
        }
        let packet = self
            .active
            .as_ref()
            .map(|(p, _, _, _)| p)
            .or_else(|| self.pending.as_ref().map(|(p, _)| p));
        deskfuse_platform::timing::record(
            "remote_input",
            Duration::ZERO,
            format!(
                "reason={reason} peer={:?} session={:?} generation={:?} pause={} received={} injected={} sequence={}",
                packet.and_then(|p| p.peer),
                packet.map(|p| &p.session),
                packet.map(|p| p.generation),
                self.pause.load(Ordering::Acquire),
                self.received,
                self.injected,
                self.sequence
            ),
        );
    }
    pub fn lease(&self) -> Option<String> {
        self.active
            .as_ref()
            .map(|(p, _, _, _)| format!("{}:{}:{}", p.peer.unwrap(), p.session, p.generation))
    }
    pub fn release(&mut self) -> bool {
        if self.pending.is_some() || self.active.is_some() {
            self.diagnostic("release");
        }
        self.pending = None;
        self.buffered.clear();
        let released = self
            .active
            .take()
            .is_none_or(|(_, _, _, injector)| injector.release_all().is_ok());
        let _ = self
            .pause
            .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                matches!(value, 4 | 5).then_some(0)
            });
        self.release_failed |= !released;
        !self.release_failed
    }
    pub fn tick(&mut self, authority: &Authority, emergency: bool) {
        if emergency {
            self.diagnostic("emergency");
            self.release();
            return;
        }
        if let Some((packet, _, _, injector)) = &self.active {
            let reason = if self.pause.load(Ordering::Acquire) != 5 {
                Some("exclusive_ownership_lost")
            } else if Instant::now() >= self.deadline {
                Some("heartbeat_timeout")
            } else if !Self::allowed(packet, authority) {
                Some("authority_revoked")
            } else if !self.platform.session_is_active() {
                Some("system_session_inactive")
            } else if !self.platform.can_post_input() {
                Some("permission_denied")
            } else if self.platform.frontmost_pid() != packet.pid {
                deskfuse_platform::timing::record(
                    "remote_foreground",
                    Duration::ZERO,
                    format!(
                        "target_pid={} foreground_pid={}",
                        packet.pid,
                        self.platform.frontmost_pid()
                    ),
                );
                Some("foreground_changed")
            } else if injector.lease_expired() {
                Some("native_lease_expired")
            } else if injector.take_error().is_some() {
                Some("native_injection_error")
            } else {
                None
            };
            if let Some(reason) = reason {
                self.diagnostic(reason);
                self.release();
            }
        }
        if let Some((packet, since)) = self.pending.take() {
            if since.elapsed() >= TTL || !Self::allowed(&packet, authority) {
                self.diagnostic("configure_timeout_or_revoked");
                self.release();
                return;
            }
            if self.pause.load(Ordering::Acquire) != 5 {
                self.pending = Some((packet, since));
                return;
            }
            if !self.platform.can_post_input() {
                self.diagnostic("permission_denied");
                self.release();
                return;
            }
            if self.platform.frontmost_pid() != packet.pid {
                self.diagnostic("foreground_changed");
                self.release();
                return;
            }
            let result = (|| {
                let id = InputSessionId(uuid::Uuid::parse_str(&packet.session).ok()?);
                let injector = self.platform.input_injector().ok()?;
                if !injector.arm_session(id, TTL) {
                    return None;
                }
                let display = DisplayDescriptor {
                    id: DisplayId("remote-application".into()),
                    frame_x: packet.x,
                    frame_y: packet.y,
                    width: packet.width,
                    height: packet.height,
                    scale_factor: 1.,
                    rotation_degrees: 0,
                    is_primary: true,
                };
                Some((packet, id, display, injector))
            })();
            if let Some(active) = result {
                self.active = Some(active);
                self.diagnostic("armed");
                self.sequence = 0;
                self.deadline = Instant::now() + TTL;
                while let Some(packet) = self.buffered.pop_front() {
                    self.receive(packet, authority);
                }
            } else {
                self.diagnostic("native_arm_failed");
                self.release();
            }
        }
    }
    pub fn receive(&mut self, packet: Packet, authority: &Authority) {
        if !Self::allowed(&packet, authority) {
            self.diagnostic("packet_unauthorized");
            return;
        }
        if packet.kind == "quiesce" {
            // A nonce-bound barrier, not an input lease. The UI waits for this
            // acknowledgement before changing the foreground process. Releases
            // queued before it have drained on this same service thread.
            if uuid::Uuid::parse_str(&packet.session).is_err()
                || self
                    .active
                    .as_ref()
                    .is_some_and(|(p, _, _, _)| p.peer != packet.peer)
                || self
                    .pending
                    .as_ref()
                    .is_some_and(|(p, _)| p.peer != packet.peer)
            {
                return;
            }
            self.quiesced = None;
            if self.release() {
                self.quiesced = Some(format!("{}:{}", packet.peer.unwrap(), packet.session));
            }
            return;
        }
        if packet.kind == "configure" {
            self.quiesced = None;
            // A second remote peer never preempts an existing controller.
            if self
                .active
                .as_ref()
                .is_some_and(|(p, _, _, _)| p.peer != packet.peer)
                || self
                    .pending
                    .as_ref()
                    .is_some_and(|(p, _)| p.peer != packet.peer)
            {
                return;
            }
            if packet.pid <= 0
                || packet.width == 0
                || packet.height == 0
                || packet.width > 32768
                || packet.height > 32768
                || !packet.x.is_finite()
                || !packet.y.is_finite()
                || uuid::Uuid::parse_str(&packet.session).is_err()
            {
                return;
            }
            self.release();
            self.request = Some(format!(
                "{}:{}:{}",
                packet.peer.unwrap(),
                packet.session,
                packet.generation
            ));
            self.error = None;
            if self
                .pause
                .compare_exchange(0, 4, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                self.pending = Some((packet, Instant::now()));
                self.diagnostic("configure_waiting_pause");
            } else {
                self.diagnostic("configure_pause_busy");
            }
            return;
        }
        if packet.kind == "release" {
            self.diagnostic("remote_release_request");
            if self
                .active
                .as_ref()
                .is_some_and(|(p, _, _, _)| Self::same(p, &packet))
                || self
                    .pending
                    .as_ref()
                    .is_some_and(|(p, _)| Self::same(p, &packet))
            {
                self.release();
            }
            return;
        }
        if let Some((pending, _)) = &self.pending {
            if Self::same(pending, &packet) {
                if self.buffered.len() < 128 {
                    self.buffered.push_back(packet);
                } else {
                    self.release();
                }
            }
            return;
        }
        let Some((owner, session, display, injector)) = &self.active else {
            return;
        };
        if !Self::same(owner, &packet) {
            return;
        }
        // Focus can change between the 50 ms watchdog ticks, especially while
        // another application's windows are being created or unminimized.
        if self.platform.frontmost_pid() != owner.pid {
            self.diagnostic("foreground_changed");
            self.release();
            return;
        }
        if packet.kind == "heartbeat" {
            if Instant::now() < self.deadline && injector.renew_deadline(TTL) {
                self.deadline = Instant::now() + TTL;
            } else {
                self.release();
            }
            return;
        }
        if Instant::now() >= self.deadline || packet.sequence <= self.sequence {
            return;
        }
        self.sequence = packet.sequence;
        self.received += 1;
        let mut events = vec![SharedInputEvent::Modifiers {
            modifiers: packet.flags,
        }];
        if matches!(packet.kind.as_str(), "move" | "down" | "up" | "scroll") {
            if !packet.x.is_finite()
                || !packet.y.is_finite()
                || !(0.0..=1.0).contains(&packet.x)
                || !(0.0..=1.0).contains(&packet.y)
            {
                return;
            }
            events.push(SharedInputEvent::Motion {
                x: (packet.x * f64::from(display.width)).min(f64::from(display.width) - 0.001),
                y: (packet.y * f64::from(display.height)).min(f64::from(display.height) - 0.001),
            });
        }
        match packet.kind.as_str() {
            "down" | "up" => events.push(SharedInputEvent::Button {
                button: packet.button,
                pressed: packet.kind == "down",
            }),
            "keyDown" | "keyUp" => events.push(SharedInputEvent::Key {
                code: packet.key,
                pressed: packet.kind == "keyDown",
                modifiers: packet.flags,
                repeat: packet.repeat,
            }),
            "scroll" => events.push(SharedInputEvent::Scroll {
                dx: packet.dx,
                dy: packet.dy,
            }),
            "move" | "flags" => {}
            _ => return,
        }
        if injector
            .enqueue_batch(*session, display, packet.generation, &events)
            .is_err()
        {
            self.diagnostic("enqueue_failed");
            self.release();
        } else {
            self.injected += 1;
            if packet.kind == "down" || packet.kind == "up" {
                self.diagnostic(&format!("pointer_{}_native_queued", packet.kind));
            }
            if self.last_log.elapsed() >= Duration::from_secs(1) || self.injected == 1 {
                self.diagnostic("input_progress");
                self.last_log = Instant::now();
            }
        }
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet(generation: u64) -> Packet {
        serde_json::from_value(serde_json::json!({
            "kind": "configure", "session": "11111111-1111-1111-1111-111111111111",
            "generation": generation, "pid": 1, "width": 1920, "height": 1080
        }))
        .unwrap()
    }
    fn authority(peer: DeviceId) -> Authority {
        Authority {
            local: DeviceId::from_bytes([1; 32]),
            network: Default::default(),
            peers: vec![PeerSnapshot {
                id: peer,
                display_name: "test".into(),
                public_key: [2; 32],
                liveness: LivenessState::Online,
                trust: TrustState::Trusted,
                connection: ConnectionState::Connected,
                routes: vec![],
                selected_route: None,
            }],
        }
    }
    #[test]
    fn quiesce_acknowledges_nonce_only_after_pending_input_is_cleared() {
        let pause = Arc::new(AtomicU8::new(4));
        let mut owner = Owner::new(pause.clone(), crate::platform::memory_platform());
        let peer = DeviceId::from_bytes([2; 32]);
        let mut pending = packet(1);
        pending.peer = Some(peer);
        owner.pending = Some((pending.clone(), Instant::now()));
        owner.buffered.push_back(pending.clone());
        let mut barrier = pending;
        barrier.kind = "quiesce".into();
        barrier.session = uuid::Uuid::new_v4().to_string();
        owner.receive(barrier.clone(), &authority(peer));
        assert!(owner.pending.is_none());
        assert!(owner.buffered.is_empty());
        assert_eq!(pause.load(Ordering::Acquire), 0);
        assert_eq!(owner.quiesced, Some(format!("{peer}:{}", barrier.session)));
        barrier.session = uuid::Uuid::new_v4().to_string();
        owner.receive(barrier.clone(), &authority(peer));
        assert_eq!(owner.quiesced, Some(format!("{peer}:{}", barrier.session)));
    }
    #[test]
    fn quiesce_never_preempts_another_peer_or_acknowledges_a_failed_release() {
        let mut owner = Owner::new(
            Arc::new(AtomicU8::new(4)),
            crate::platform::memory_platform(),
        );
        let peer = DeviceId::from_bytes([2; 32]);
        let mut pending = packet(1);
        pending.peer = Some(DeviceId::from_bytes([3; 32]));
        owner.pending = Some((pending, Instant::now()));
        let mut barrier = packet(1);
        barrier.peer = Some(peer);
        barrier.kind = "quiesce".into();
        owner.receive(barrier.clone(), &authority(peer));
        assert!(owner.pending.is_some());
        assert!(owner.quiesced.is_none());
        owner.pending = None;
        owner.release_failed = true;
        owner.receive(barrier, &authority(peer));
        assert!(owner.quiesced.is_none());
    }
    #[test]
    fn quiesce_requires_authority_and_a_valid_nonce() {
        let mut owner = Owner::new(
            Arc::new(AtomicU8::new(0)),
            crate::platform::memory_platform(),
        );
        let peer = DeviceId::from_bytes([2; 32]);
        let mut barrier = packet(1);
        barrier.peer = Some(peer);
        barrier.kind = "quiesce".into();
        let mut denied = authority(peer);
        denied.network.blocked_peer_ids.push(peer);
        owner.receive(barrier.clone(), &denied);
        assert!(owner.quiesced.is_none());
        barrier.session = "invalid".into();
        owner.receive(barrier, &authority(peer));
        assert!(owner.quiesced.is_none());
    }
    #[test]
    fn remote_release_never_unpauses_viewer_or_update() {
        for value in [1, 2, 3, 6, 7] {
            let pause = Arc::new(AtomicU8::new(value));
            let mut owner = Owner::new(pause.clone(), crate::platform::memory_platform());
            owner.release();
            assert_eq!(pause.load(Ordering::Acquire), value);
        }
        for value in [4, 5] {
            let pause = Arc::new(AtomicU8::new(value));
            Owner::new(pause.clone(), crate::platform::memory_platform()).release();
            assert_eq!(pause.load(Ordering::Acquire), 0);
        }
    }
    #[test]
    fn emergency_cancels_pending_handshake_and_buffered_input() {
        let pause = Arc::new(AtomicU8::new(4));
        let mut owner = Owner::new(pause.clone(), crate::platform::memory_platform());
        owner.pending = Some((packet(1), Instant::now()));
        owner.buffered.push_back(packet(1));
        let authority = Authority {
            local: DeviceId::from_bytes([1; 32]),
            network: Default::default(),
            peers: vec![],
        };
        owner.tick(&authority, true);
        assert!(owner.pending.is_none());
        assert!(owner.buffered.is_empty());
        assert_eq!(pause.load(Ordering::Acquire), 0);
    }
    #[test]
    fn surface_and_peer_both_bind_input_lease() {
        let mut a = packet(1);
        let mut b = packet(1);
        a.peer = Some(DeviceId::from_bytes([1; 32]));
        b.peer = a.peer;
        assert!(Owner::same(&a, &b));
        b.generation = 2;
        assert!(!Owner::same(&a, &b));
        b.generation = 1;
        b.peer = Some(DeviceId::from_bytes([2; 32]));
        assert!(!Owner::same(&a, &b));
    }
}
