//! The input helper owns shortcut routing even before an audio reader exists.
use deskfuse_core::{DeviceId, InputShareRole};
use deskfuse_platform_macos::sharing::microphone_shortcut::{self as native, ShortcutConfig};
use deskfuse_runtime::{
    RuntimeHandle, input_service::Authority, input_service_types::RemoteApplicationInput,
};
use deskfuse_transport::{QuinnTransport, microphone_shortcut::ShortcutSession};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::watch;

fn diagnostic(stage: &'static str, detail: String) {
    deskfuse_platform::timing::record(stage, Duration::ZERO, detail);
}

pub struct Owner {
    local: DeviceId,
    transport: QuinnTransport,
    holds: BTreeMap<u64, watch::Sender<bool>>,
    allowed: BTreeSet<DeviceId>,
    authority: Option<(bool, InputShareRole, Option<DeviceId>)>,
    injection: Arc<AtomicBool>,
    tasks: tokio::task::JoinSet<()>,
}
impl Owner {
    pub fn new(local: DeviceId, transport: QuinnTransport) -> Self {
        Self {
            local,
            transport,
            holds: BTreeMap::new(),
            allowed: BTreeSet::new(),
            authority: None,
            injection: Arc::new(AtomicBool::new(false)),
            tasks: tokio::task::JoinSet::new(),
        }
    }
    pub fn configure(&mut self, packet: &RemoteApplicationInput) {
        native::configure(ShortcutConfig {
            enabled: packet.repeat,
            key_code: packet.key,
            modifiers: packet.flags,
        });
    }
    pub fn tick(&mut self, authority: &Authority, runtime: &RuntimeHandle) {
        let snapshot = runtime.input_snapshot();
        let effective = snapshot.effective_settings();
        let allowed: BTreeSet<_> = snapshot
            .links
            .iter()
            .filter(|link| link.ready && link.control_session.is_some())
            .map(|link| link.peer_id)
            .filter(|peer| {
                authority.network.allows(self.local, *peer)
                    && !authority.network.blocked_peer_ids.contains(peer)
                    && authority
                        .peers
                        .iter()
                        .any(|p| p.id == *peer && p.trust == deskfuse_core::TrustState::Trusted)
            })
            .collect();
        // Pointer crossings retain the pressed target. Authority changes do
        // not: a hold must never outlive its input role or controller grant.
        let authority = (
            effective.enabled,
            effective.role,
            effective.authorized_controller,
        );
        if allowed != self.allowed || self.authority != Some(authority) {
            native::cancel();
            self.allowed = allowed;
            self.authority = Some(authority);
        }
        for edge in native::take_edges() {
            if !edge.pressed {
                if let Some(tx) = self.holds.remove(&edge.serial) {
                    let _ = tx.send(false);
                }
                continue;
            }
            if edge.generation != native::generation()
                || (edge.target != self.local && !self.allowed.contains(&edge.target))
            {
                continue;
            }
            let (tx, rx) = watch::channel(true);
            self.holds.insert(edge.serial, tx);
            let transport = self.transport.clone();
            let local = self.local;
            let injection = self.injection.clone();
            self.tasks.spawn(async move {
                send_hold(
                    HoldContext {
                        transport,
                        local,
                        occupied: injection,
                    },
                    edge.target,
                    edge.config,
                    edge.routed,
                    edge.generation,
                    rx,
                )
                .await;
            });
        }
        self.holds.retain(|_, tx| !tx.is_closed());
        for _ in 0..8 {
            let Some(session) = self.transport.take_microphone_shortcut() else {
                break;
            };
            let config = native::config();
            if !config.enabled
                || !effective.enabled
                || !self.allowed.contains(&session.peer)
                || session.request.key_code != u32::from(config.key_code)
                || session.request.modifiers != config.modifiers
            {
                diagnostic(
                    "microphone_shortcut_rejected",
                    format!(
                        "peer={} enabled={} input_enabled={} allowed={} key_match={} modifiers_match={}",
                        session.peer,
                        config.enabled,
                        effective.enabled,
                        self.allowed.contains(&session.peer),
                        session.request.key_code == u32::from(config.key_code),
                        session.request.modifiers == config.modifiers
                    ),
                );
                continue;
            }
            let target = if session.request.routed {
                if effective.role != InputShareRole::Controlled
                    || effective.authorized_controller != Some(session.peer)
                {
                    diagnostic(
                        "microphone_shortcut_rejected",
                        format!("peer={} reason=controller_authorization", session.peer),
                    );
                    continue;
                }
                self.local
            } else {
                if effective.role != InputShareRole::Controller {
                    diagnostic(
                        "microphone_shortcut_rejected",
                        format!("peer={} reason=not_controller", session.peer),
                    );
                    continue;
                }
                let Some(target) = native::current_target() else {
                    continue;
                };
                target
            };
            if target != self.local && !self.allowed.contains(&target) {
                continue;
            }
            if self.tasks.len() >= 16 {
                continue;
            }
            diagnostic(
                "microphone_shortcut_received",
                format!(
                    "peer={} target={} routed={}",
                    session.peer, target, session.request.routed
                ),
            );
            let transport = self.transport.clone();
            let local = self.local;
            let injection = self.injection.clone();
            let generation = native::generation();
            self.tasks.spawn(async move {
                receive_hold(
                    transport, local, target, config, generation, session, injection,
                )
                .await;
            });
        }
        while self.tasks.try_join_next().is_some() {}
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        native::configure(ShortcutConfig::default());
        self.tasks.abort_all();
    }
}
struct Held {
    _native: native::Injection,
    occupied: Arc<AtomicBool>,
}
impl Held {
    fn new(config: ShortcutConfig, occupied: Arc<AtomicBool>) -> Option<Self> {
        if occupied.swap(true, Ordering::AcqRel) {
            diagnostic("microphone_shortcut_busy", "已有快捷键按住会话".into());
            return None;
        }
        match native::Injection::press(config) {
            Ok(injection) => Some(Self {
                _native: injection,
                occupied,
            }),
            Err(error) => {
                diagnostic("microphone_shortcut_injection_failed", error);
                occupied.store(false, Ordering::Release);
                None
            }
        }
    }
}
impl Drop for Held {
    fn drop(&mut self) {
        self.occupied.store(false, Ordering::Release);
    }
}
fn valid(generation: u64) -> bool {
    native::generation() == generation && native::current_target().is_some()
}
struct HoldContext {
    transport: QuinnTransport,
    local: DeviceId,
    occupied: Arc<AtomicBool>,
}

async fn send_hold(
    context: HoldContext,
    target: DeviceId,
    config: ShortcutConfig,
    routed: bool,
    generation: u64,
    mut held: watch::Receiver<bool>,
) {
    let HoldContext {
        transport,
        local,
        occupied,
    } = context;
    if !*held.borrow() || !valid(generation) {
        return;
    }
    let mut remote = if target != local {
        match transport
            .open_microphone_shortcut(target, config.key_code, config.modifiers, routed)
            .await
        {
            Ok(s) => Some(s),
            Err(error) => {
                diagnostic(
                    "microphone_shortcut_open_failed",
                    format!("target={target} error={error}"),
                );
                return;
            }
        }
    } else {
        None
    };
    // A release received during connection negotiation must never become a late press.
    if !*held.borrow() || !valid(generation) {
        return;
    }
    let _injection = if remote.is_none() {
        match Held::new(config, occupied) {
            Some(h) => Some(h),
            None => return,
        }
    } else {
        None
    };
    diagnostic(
        "microphone_shortcut_active",
        format!("target={target} remote={}", remote.is_some()),
    );
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    loop {
        if !*held.borrow() || !valid(generation) {
            break;
        }
        tokio::select! {
            result=held.changed()=>{if result.is_err(){break;}}
            _ = tick.tick() => {
                if let Some(session) = &mut remote
                    && let Err(error) = session.send_held(true).await
                {
                    diagnostic("microphone_shortcut_send_failed", format!("target={target} error={error}"));
                    break;
                }
            }
        }
    }
    if let Some(session) = &mut remote
        && let Err(error) = session.send_held(false).await
    {
        diagnostic(
            "microphone_shortcut_release_failed",
            format!("target={target} error={error}"),
        );
    }
    diagnostic("microphone_shortcut_ended", format!("target={target}"));
}
async fn receive_hold(
    transport: QuinnTransport,
    local: DeviceId,
    target: DeviceId,
    config: ShortcutConfig,
    generation: u64,
    mut incoming: ShortcutSession,
    occupied: Arc<AtomicBool>,
) {
    if !valid(generation) || !matches!(incoming.receive_held().await, Ok(true)) {
        return;
    }
    let (tx, rx) = watch::channel(true);
    let outgoing = send_hold(
        HoldContext {
            transport,
            local,
            occupied,
        },
        target,
        config,
        true,
        generation,
        rx,
    );
    complete_hold_on_release(outgoing, tx, async {
        tokio::select! {
            _=async {while matches!(incoming.receive_held().await,Ok(true)) {}}=>{},
            _=async {while valid(generation){tokio::time::sleep(Duration::from_millis(100)).await;}}=>{},
        }
    }).await;
}

async fn complete_hold_on_release(
    outgoing: impl std::future::Future<Output = ()>,
    held: watch::Sender<bool>,
    released: impl std::future::Future<Output = ()>,
) {
    tokio::pin!(outgoing);
    // Drain a buffered release before local injection or a just-completed
    // downstream negotiation. A finished hold must not briefly press a key.
    tokio::select! {
        biased;
        _ = released => {},
        _ = &mut outgoing => return,
    }
    let _ = held.send(false);
    outgoing.await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shortcut_buffered_release_prevents_late_local_press() {
        let (held, observed) = watch::channel(true);
        let pressed = AtomicBool::new(false);
        let outgoing = async {
            if *observed.borrow() {
                pressed.store(true, Ordering::SeqCst);
            }
        };
        complete_hold_on_release(outgoing, held, std::future::ready(())).await;
        assert!(!pressed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn shortcut_release_during_downstream_negotiation_prevents_late_press() {
        let (held, observed) = watch::channel(true);
        let (started, waiting) = tokio::sync::oneshot::channel();
        let (connected, connection) = tokio::sync::oneshot::channel();
        let pressed = AtomicBool::new(false);
        let outgoing = async {
            started.send(()).unwrap();
            connection.await.unwrap();
            if *observed.borrow() {
                pressed.store(true, Ordering::SeqCst);
            }
        };
        let released = async {
            waiting.await.unwrap();
            connected.send(()).unwrap();
        };
        complete_hold_on_release(outgoing, held, released).await;
        assert!(!pressed.load(Ordering::SeqCst));
    }
}
