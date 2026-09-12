//! Input has its own actor and clock. Clipboard RPC waits cannot delay lease expiry.
use crate::input_delivery::InputDelivery;
use crate::{RuntimeCommand, RuntimeError, RuntimeEvent, RuntimeStore, TransportProvider};
use futures::StreamExt;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, mpsc, watch};
use unidesk_core::*;
use unidesk_transport::InputTransportEvent;
mod activation;
mod environment;
mod hybrid;
mod layout;
mod recovery;
use activation::{Activation, RetiredActivation};
use hybrid::HybridLease;
const TTL: Duration = Duration::from_millis(1500);
const RENEW: Duration = Duration::from_millis(250);
// Bound work per select turn so motion bursts cannot starve lease/network processing.
const MAX_MOTION_COMMAND_DRAIN: usize = 64;

#[derive(Clone, Debug)]
struct Lease {
    activation_id: InputActivationId,
    peer: DeviceId,
    context: ConnectionContext,
    token: Option<InputToken>,
    controller: bool,
    target: DisplayId,
    entered: bool,
    sequence: u64,
    deadline: Instant,
    continuous_deadline_ms: u64,
    renew_at: Instant,
    awaiting_ack: bool,
    revision: u64,
    hybrid: HybridLease,
}
impl Lease {
    fn expired(&self) -> bool {
        Instant::now() >= self.deadline || lease_now_ms() >= self.continuous_deadline_ms
    }
    fn extend(&mut self) {
        self.deadline = Instant::now() + TTL;
        self.continuous_deadline_ms = lease_now_ms().checked_add(1500).unwrap_or(0);
    }
}
fn lease_now_ms() -> u64 {
    #[cfg(target_os = "macos")]
    {
        let clock = nix::time::ClockId::from_raw(nix::libc::CLOCK_MONOTONIC_RAW);
        nix::time::clock_gettime(clock)
            .ok()
            .and_then(|t| {
                u64::try_from(t.tv_sec())
                    .ok()
                    .and_then(|s| s.checked_mul(1000))
                    .and_then(|s| s.checked_add(u64::try_from(t.tv_nsec()).ok()? / 1_000_000))
            })
            .unwrap_or(u64::MAX)
    }
    #[cfg(not(target_os = "macos"))]
    {
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        START
            .get_or_init(Instant::now)
            .elapsed()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }
}

struct InputActor {
    links: recovery::Links,
    environment: environment::Environment,
    outgoing: BTreeMap<DeviceId, Activation>,
    incoming: Option<Activation>,
    retired_activations: std::collections::VecDeque<RetiredActivation>,
    paused_intents: std::collections::BTreeSet<(DeviceId, InputActivationId)>,
    controller_intents: BTreeMap<DeviceId, InputActivationId>,
    activation_policy_error: Option<String>,
    pending_policy_save: Option<tokio::task::JoinHandle<std::result::Result<(), RuntimeError>>>,
    recent_managed_intent: Option<(DeviceId, InputActivationId)>,
    snapshot: InputSharingSnapshot,
    network: watch::Receiver<crate::SnapshotView>,
    membership: Option<crate::DeviceNetworkSnapshot>,
    transport: Arc<dyn TransportProvider>,
    store: Arc<dyn RuntimeStore>,
    events: broadcast::Sender<RuntimeEvent>,
    delivery: Arc<InputDelivery>,
    output: watch::Sender<InputSharingSnapshot>,
    available: bool,
    unavailable_reason: Option<String>,
    leases: BTreeMap<InputSessionId, Lease>,
    published_topologies: BTreeMap<(DeviceId, TransportSessionId, u64), Instant>,
    attempted: BTreeMap<DeviceId, Instant>,
    motion_probe_started: BTreeMap<TransportSessionId, Instant>,
    motion_disabled_until: BTreeMap<DeviceId, Instant>,
    started: Instant,
    revision: u64,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    mut commands: mpsc::Receiver<RuntimeCommand>,
    output: watch::Sender<InputSharingSnapshot>,
    network: watch::Receiver<crate::SnapshotView>,
    transport: Arc<dyn TransportProvider>,
    store: Arc<dyn RuntimeStore>,
    events: broadcast::Sender<RuntimeEvent>,
    overflow: Arc<AtomicBool>,
    delivery: Arc<InputDelivery>,
) {
    let load = store.clone();
    let settings = tokio::task::spawn_blocking(move || load.get_input_sharing_settings())
        .await
        .ok()
        .and_then(|r| r.ok())
        .filter(|s| s.validate().is_ok())
        .unwrap_or_default();
    let load = store.clone();
    let scenes = tokio::task::spawn_blocking(move || load.get_input_sharing_scenes())
        .await
        .ok()
        .and_then(|r| r.ok())
        .filter(|v| InputSharingScene::validate_list(v).is_ok())
        .unwrap_or_default();
    let mut inbox = transport.input_messages();
    let mut actor = InputActor {
        links: recovery::Links::default(),
        environment: environment::Environment::default(),
        outgoing: BTreeMap::new(),
        incoming: None,
        retired_activations: std::collections::VecDeque::new(),
        paused_intents: std::collections::BTreeSet::new(),
        controller_intents: BTreeMap::new(),
        activation_policy_error: None,
        pending_policy_save: None,
        recent_managed_intent: None,
        snapshot: InputSharingSnapshot {
            settings,
            scenes,
            ..Default::default()
        },
        network,
        membership: None,
        transport,
        store,
        events,
        delivery,
        output,
        available: false,
        unavailable_reason: Some("Input permissions or active login session unavailable".into()),
        leases: BTreeMap::new(),
        published_topologies: BTreeMap::new(),
        attempted: BTreeMap::new(),
        motion_probe_started: BTreeMap::new(),
        motion_disabled_until: BTreeMap::new(),
        started: Instant::now(),
        revision: 0,
    };
    actor.load_activation_policy().await;
    actor.emit(RuntimeEvent::InputSharingSettingsChanged(
        actor.snapshot.settings.clone(),
    ));
    let mut interval = tokio::time::interval(Duration::from_millis(50));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut environment_save = tokio::time::interval(Duration::from_secs(2));
    let mut pending_command = None;
    loop {
        let motion_deadline = actor.next_motion_snapshot_deadline();
        // Fail closed before processing another event, not only at a maintenance tick.
        if overflow.swap(false, Ordering::AcqRel) {
            actor.deactivate_all("input delivery overflow");
        }
        let mut publish_snapshot = true;
        let lease_count = actor.leases.len();
        tokio::select! {
            _ = environment_save.tick() => actor.start_environment_save(),
            _ = async { match motion_deadline {Some(deadline)=>tokio::time::sleep_until(deadline.into()).await,None=>std::future::pending::<()>().await} } => actor.tick_motion(),
            command = async {
                match pending_command.take() {
                    Some(command) => Some(command),
                    None => commands.recv().await,
                }
            } => match command {
                Some(RuntimeCommand::Shutdown) => break,
                Some(command) => {
                    let command = actor.coalesce_queued_motion(command, &mut commands, &mut pending_command);
                    publish_snapshot = !matches!(&command, RuntimeCommand::SendSharedInput { events, .. }
                        if !events.is_empty() && events.iter().all(|event| matches!(event, SharedInputEvent::Motion { .. })));
                    actor.command(command).await;
                }
                None => break,
            },
            event = inbox.next() => if let Some(event) = event {
                publish_snapshot = !matches!(&event, InputTransportEvent::Motion { .. });
                actor.inbound(event);
            } else { break },
            _=interval.tick()=>{if overflow.swap(false,Ordering::AcqRel){actor.deactivate_all("input command queue overflow");}actor.tick();},
            changed=actor.network.changed()=>{if changed.is_err(){break}actor.tick();},
        }
        // Do not wait on disk I/O or cancellation cleanup between mouse samples.
        if actor
            .pending_policy_save
            .as_ref()
            .is_some_and(|task| task.is_finished())
        {
            actor.finish_policy_save().await;
        }
        actor.reap_links().await;
        // Motion is delivered directly to the native worker, not the UI. Avoid
        // rebuilding/cloning every peer and transfer snapshot on every sample.
        // Releases still publish immediately; maintenance refreshes diagnostics.
        if publish_snapshot || actor.leases.len() != lease_count {
            actor.publish();
        }
    }
    actor.finish_policy_save().await;
    actor.persist_environment_if_dirty().await;
    actor.stop_links().await;
    actor.deactivate_all("input runtime stopped");
}
impl InputActor {
    /// Only valid absolute motion may be superseded. In particular, retaining an
    /// invalid/out-of-display frame lets the normal command path revoke its lease.
    fn motion_command_target(
        &self,
        command: &RuntimeCommand,
    ) -> Option<(InputSessionId, DisplayId)> {
        let RuntimeCommand::SendSharedInput {
            session_id,
            target_display,
            events,
        } = command
        else {
            return None;
        };
        let lease = self.leases.get(session_id)?;
        let topology = self.snapshot.remote_topologies.get(&lease.peer)?;
        let display = topology.displays.iter().find(|d| d.id == *target_display)?;
        if events.is_empty() || events.len() > 256 || events.iter().any(|event| {
            event.validate().is_err()
                || !matches!(event, SharedInputEvent::Motion { x, y } if display.contains(*x, *y))
        }) {
            return None;
        }
        Some((*session_id, target_display.clone()))
    }

    fn coalesce_queued_motion(
        &self,
        mut command: RuntimeCommand,
        commands: &mut mpsc::Receiver<RuntimeCommand>,
        pending: &mut Option<RuntimeCommand>,
    ) -> RuntimeCommand {
        let Some(target) = self.motion_command_target(&command) else {
            return command;
        };
        for _ in 1..MAX_MOTION_COMMAND_DRAIN {
            let Ok(next) = commands.try_recv() else {
                break;
            };
            if self.motion_command_target(&next).as_ref() != Some(&target) {
                // Never merge across a click, scroll, key, configuration or session
                // boundary. The one look-ahead command remains first next turn.
                *pending = Some(next);
                break;
            }
            command = next;
        }
        if let RuntimeCommand::SendSharedInput { events, .. } = &mut command {
            // All positions were validated above; these are absolute coordinates,
            // not relative deltas. Replaying older positions only adds latency.
            if events.len() > 1 {
                let latest = events.pop().unwrap();
                events.clear();
                events.push(latest);
            }
        }
        command
    }

    fn emit(&self, event: RuntimeEvent) {
        // Ordered native delivery is not coupled to a slow GPUI/UI subscriber.
        self.delivery.emit(event.clone());
        if !matches!(event, RuntimeEvent::SharedInputReceived { .. }) {
            let _ = self.events.send(event);
        }
    }
    fn error(&self, message: impl Into<String>) {
        self.emit(RuntimeEvent::Error(RuntimeError::InvalidCommand(
            message.into(),
        )));
    }
    fn network_allowed(&self, peer: DeviceId) -> bool {
        let view = self.network.borrow();
        view.device_network
            .allows(view.snapshot.local_device_id, peer)
            && view.snapshot.peers.iter().any(|p| {
                p.id == peer
                    && p.trust == TrustState::Trusted
                    && p.connection == ConnectionState::Connected
                    && p.routes
                        .iter()
                        .any(|r| r.capabilities.contains(Capabilities::INPUT_SHARING))
            })
    }
    fn role_allowed(&self, peer: DeviceId, controller: bool) -> bool {
        let s = &self.snapshot.settings;
        self.available
            && if controller {
                s.enabled
                    && s.role == InputShareRole::Controller
                    && !self.snapshot.environment.local_suspended
                    && s.participants.contains(&peer)
                    && self.outgoing.get(&peer).is_some_and(|a| a.ready())
            } else {
                self.incoming
                    .as_ref()
                    .is_some_and(|a| a.peer == peer && a.ready())
            }
    }
    fn context_live(&self, peer: DeviceId, context: &ConnectionContext) -> bool {
        context.input_eligible()
            && self
                .transport
                .input_contexts(peer)
                .iter()
                .any(|c| c == context)
    }
    fn send(
        &self,
        peer: DeviceId,
        context: &ConnectionContext,
        message: InputShareMessage,
    ) -> bool {
        if !self.network_allowed(peer) || !self.context_live(peer, context) {
            return false;
        }
        self.transport
            .send_input(peer, context.session_id, message)
            .is_ok()
    }
    fn publish_topology(&mut self, peer: DeviceId, context: &ConnectionContext, now: Instant) {
        let Some(topology) = self.snapshot.local_topology.clone() else {
            return;
        };
        let key = (peer, context.session_id, topology.generation);
        if self
            .published_topologies
            .get(&key)
            .is_none_or(|sent| now.duration_since(*sent) >= Duration::from_secs(2))
            && self.send(peer, context, InputShareMessage::DisplayTopology(topology))
        {
            self.published_topologies.insert(key, now);
        }
    }
    fn emit_release(&self, id: InputSessionId, peer: DeviceId, reason: &str) {
        self.emit(RuntimeEvent::InputControlReleased {
            peer_id: peer,
            session_id: id,
            reason: reason.into(),
        });
    }
    fn release(&mut self, id: InputSessionId, reason: &str, notify: bool) {
        if let Some(lease) = self.leases.remove(&id) {
            self.transport
                .cancel_input_motion(lease.peer, lease.context.session_id, id);
            if notify && let Some(token) = lease.token {
                self.send(
                    lease.peer,
                    &lease.context,
                    InputShareMessage::Release {
                        session_id: id,
                        token,
                        sequence: lease.sequence.saturating_add(1),
                    },
                );
            }
            self.emit_release(id, lease.peer, reason);
        }
    }
    fn release_all(&mut self, reason: &str) {
        let ids = self.leases.keys().copied().collect::<Vec<_>>();
        for id in ids {
            self.release(id, reason, true)
        }
    }
    fn revision(&mut self) -> u64 {
        self.revision = self.revision.saturating_add(1);
        self.revision
    }
    fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
    fn request(&mut self, peer: DeviceId, target: DisplayId) {
        if self.leases.values().any(|l| l.peer == peer) {
            return;
        }
        let Some(context) = self
            .outgoing
            .get(&peer)
            .filter(|a| a.ready())
            .and_then(|a| a.context.clone())
        else {
            self.emit(RuntimeEvent::InputLinkUnavailable {
                peer_id: peer,
                reason: "No verified physical on-link LAN session".into(),
            });
            return;
        };
        if !self.role_allowed(peer, true) || !self.network_allowed(peer) {
            return;
        }
        if !self
            .snapshot
            .remote_topologies
            .get(&peer)
            .is_some_and(|t| t.displays.iter().any(|d| d.id == target))
        {
            return;
        }
        let id = InputSessionId::new();
        let supports_motion = self
            .transport
            .input_capabilities(peer, context.session_id)
            .contains(Capabilities::INPUT_MOTION_DATAGRAM_V1)
            && !self
                .motion_disabled_until
                .get(&peer)
                .is_some_and(|until| *until > Instant::now());
        let motion_available = self
            .transport
            .input_motion_available(peer, context.session_id);
        if supports_motion && !motion_available {
            let started = self
                .motion_probe_started
                .entry(context.session_id)
                .or_insert_with(Instant::now);
            if started.elapsed() < Duration::from_secs(1) {
                // Retry from the normal actor tick instead of waiting a full topology retry.
                self.attempted.remove(&peer);
                return;
            }
        }
        let hybrid_request = supports_motion && motion_available;
        let request = if hybrid_request {
            InputShareMessage::RequestHybrid {
                session_id: id,
                target_display: target.clone(),
            }
        } else {
            InputShareMessage::Request {
                session_id: id,
                target_display: target.clone(),
            }
        };
        if !self.send(peer, &context, request) {
            return;
        }
        self.leases.insert(
            id,
            Lease {
                activation_id: self.outgoing[&peer].id,
                peer,
                context,
                token: None,
                controller: true,
                target,
                entered: false,
                sequence: 0,
                deadline: Instant::now() + TTL,
                continuous_deadline_ms: lease_now_ms().checked_add(1500).unwrap_or(0),
                renew_at: Instant::now() + RENEW,
                awaiting_ack: true,
                revision: 0,
                hybrid: HybridLease {
                    requested: hybrid_request,
                    fallback_reason: if hybrid_request {
                        None
                    } else if self
                        .motion_disabled_until
                        .get(&peer)
                        .is_some_and(|until| *until > Instant::now())
                    {
                        Some("Datagram health failure cooldown".into())
                    } else if !supports_motion {
                        Some("Motion capability not negotiated (disabled or unsupported)".into())
                    } else {
                        Some("Datagram path probe unavailable after timeout".into())
                    },
                    ..Default::default()
                },
            },
        );
    }
    async fn save_scenes(&mut self, scenes: Vec<InputSharingScene>) {
        if let Err(e) = InputSharingScene::validate_list(&scenes) {
            self.error(e.to_string());
            return;
        }
        let store = self.store.clone();
        let persisted = scenes.clone();
        if matches!(
            tokio::task::spawn_blocking(move || store.set_input_sharing_scenes(persisted)).await,
            Ok(Ok(()))
        ) {
            self.snapshot.scenes = scenes;
        } else {
            self.error("Input scenes could not be persisted");
        }
    }
    fn settings_error(&self, error: impl Into<String>) {
        self.error(error);
        self.emit(RuntimeEvent::InputSharingSettingsChanged(
            self.snapshot.settings.clone(),
        ));
    }
    async fn command(&mut self, command: RuntimeCommand) {
        let command = match command {
            RuntimeCommand::ApplyInputSharingScene { scene_id } => {
                let Some(scene) = self.snapshot.scenes.iter().find(|s| s.id == scene_id) else {
                    self.settings_error("Unknown input scene");
                    return;
                };
                RuntimeCommand::SetInputSharingSettings(scene.settings.clone())
            }
            command => command,
        };
        match command {
            RuntimeCommand::MoveInputDevice {
                device_id,
                display_id,
                x,
                y,
            } => {
                self.move_layout(device_id, display_id, x, y).await;
            }
            RuntimeCommand::ObserveWifiEnvironment { ssid } => self.observe_environment(ssid),
            RuntimeCommand::ChooseEnvironmentController {
                controller,
                remember,
            } => {
                self.choose_environment_controller(controller, remember)
                    .await
            }
            RuntimeCommand::ForgetControllerEnvironment { ssid } => {
                self.forget_environment_rule(ssid).await
            }
            RuntimeCommand::SetManagedInputAvailability {
                activation_id,
                status,
                reason,
            } => {
                self.set_managed_availability(activation_id, status, reason);
            }
            RuntimeCommand::PauseManagedActivation { activation_id } => {
                self.pause_managed(activation_id).await;
            }
            RuntimeCommand::PauseCurrentManagedActivation => {
                self.pause_current_managed().await;
            }
            RuntimeCommand::SaveInputSharingScene { name } => {
                match InputSharingScene::new(name, self.snapshot.settings.clone()) {
                    Ok(scene) => {
                        let mut scenes = self.snapshot.scenes.clone();
                        scenes.push(scene);
                        self.save_scenes(scenes).await;
                    }
                    Err(e) => self.settings_error(e.to_string()),
                }
            }
            RuntimeCommand::RenameInputSharingScene { scene_id, name } => {
                let mut scenes = self.snapshot.scenes.clone();
                if let Some(scene) = scenes.iter_mut().find(|s| s.id == scene_id) {
                    scene.name = name.trim().to_owned();
                    self.save_scenes(scenes).await;
                } else {
                    self.settings_error("Unknown input scene");
                }
            }
            RuntimeCommand::DeleteInputSharingScene { scene_id } => {
                let mut scenes = self.snapshot.scenes.clone();
                scenes.retain(|s| s.id != scene_id);
                self.save_scenes(scenes).await;
            }
            RuntimeCommand::SetInputSharingSettings(settings) => {
                if let Err(e) = settings.validate() {
                    self.settings_error(e.to_string());
                    return;
                }
                let local = self.network.borrow().snapshot.local_device_id;
                if settings.participants.contains(&local)
                    || settings.authorized_controller == Some(local)
                {
                    self.settings_error("Input sharing cannot authorize this same device");
                    return;
                }
                self.release_all("input settings changed");
                self.finish_policy_save().await;
                if self.activation_policy_error.is_some() {
                    self.settings_error(
                        "Activation policy could not be loaded; refusing to replace it",
                    );
                    return;
                }
                let policy = self.policy_for_settings(&settings);
                let store = self.store.clone();
                let persisted = settings.clone();
                let persisted_policy = policy.clone();
                match tokio::task::spawn_blocking(move || {
                    store.set_input_sharing_settings_and_policy(persisted, persisted_policy)
                })
                .await
                {
                    Ok(Ok(())) => {
                        self.controller_intents = policy.controller_intents;
                        if settings.enabled && settings.role == InputShareRole::Controller {
                            self.end_incoming("local controller enabled", true);
                        }
                        self.snapshot.settings = settings.clone();
                        self.published_topologies.clear();
                        self.attempted.clear();
                        self.emit(RuntimeEvent::InputSharingSettingsChanged(settings));
                    }
                    _ => self.settings_error("Input settings could not be persisted"),
                }
            }
            RuntimeCommand::SetInputAvailability { available, reason } => {
                if available && self.incoming.is_some() {
                    return;
                }
                if self.available != available || self.unavailable_reason != reason {
                    self.available = available;
                    self.unavailable_reason = reason;
                    if !available {
                        self.release_all("input permissions or active session unavailable");
                        if let Some(managed) = self.incoming.as_ref().filter(|a| !a.suppressed) {
                            self.set_managed_availability(
                                managed.id,
                                InputActivationStatus::Unavailable,
                                self.unavailable_reason.clone(),
                            );
                        }
                    }
                    self.attempted.clear();
                }
            }
            RuntimeCommand::PublishDisplayTopology(topology) => {
                if let Err(e) = topology.validate() {
                    self.error(e.to_string());
                    return;
                }
                if self.snapshot.local_topology.as_ref() != Some(&topology) {
                    self.release_all("display topology changed");
                    self.snapshot.local_topology = Some(topology);
                    self.published_topologies.clear();
                }
            }
            RuntimeCommand::RequestInputControl {
                peer_id,
                target_display,
            } => self.request(peer_id, target_display),
            RuntimeCommand::EnterInputControl {
                session_id,
                target_display,
                x,
                y,
            } => {
                let valid = self.leases.get(&session_id).is_some_and(|l| {
                    l.controller
                        && l.token.is_some()
                        && !l.expired()
                        && self.role_allowed(l.peer, true)
                        && self.context_live(l.peer, &l.context)
                        && self.lease_activation_live(l)
                        && self
                            .snapshot
                            .remote_topologies
                            .get(&l.peer)
                            .is_some_and(|t| {
                                t.displays
                                    .iter()
                                    .any(|d| d.id == target_display && d.contains(x, y))
                            })
                });
                if !valid {
                    self.release(session_id, "input entry is no longer valid", true);
                    return;
                }
                let others = self
                    .leases
                    .iter()
                    .filter(|(id, l)| **id != session_id && l.controller && l.entered)
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>();
                for id in others {
                    self.leave(id)
                }
                if self.leases[&session_id].hybrid.enabled {
                    self.hybrid_enter(session_id, target_display, x, y);
                    self.tick();
                    return;
                }
                let lease = self.leases.get_mut(&session_id).unwrap();
                lease.sequence = lease.sequence.saturating_add(1);
                lease.entered = true;
                lease.target = target_display.clone();
                let message = InputShareMessage::Enter {
                    session_id,
                    token: lease.token.unwrap(),
                    target_display,
                    x,
                    y,
                    sequence: lease.sequence,
                };
                let peer = lease.peer;
                let context = lease.context.clone();
                if !self.send(peer, &context, message) {
                    self.release(session_id, "input enter send failed", true)
                }
            }
            RuntimeCommand::SendSharedInput {
                session_id,
                target_display,
                events,
            } => {
                let Some(lease) = self.leases.get(&session_id) else {
                    return;
                };
                if !lease.controller
                    || !lease.entered
                    || lease.target != target_display
                    || lease.expired()
                    || !self.role_allowed(lease.peer, true)
                    || !self.lease_activation_live(lease)
                {
                    self.release(session_id, "input frame without active lease", true);
                    return;
                }
                let Some(topology) = self.snapshot.remote_topologies.get(&lease.peer) else {
                    return;
                };
                let generation = topology.generation;
                if events.is_empty()||events.len()>256||events.iter().any(|e|e.validate().is_err() || matches!(e,SharedInputEvent::Motion{x,y} if !topology.displays.iter().any(|d|d.id==target_display && d.contains(*x,*y)))){self.release(session_id,"invalid input frame",true);return}
                if self.leases[&session_id].hybrid.enabled {
                    self.hybrid_events(session_id, events);
                    self.tick();
                    return;
                }
                let lease = self.leases.get_mut(&session_id).unwrap();
                lease.sequence = lease.sequence.saturating_add(1);
                let peer = lease.peer;
                let context = lease.context.clone();
                let message = InputShareMessage::Frame {
                    session_id,
                    token: lease.token.unwrap(),
                    target_display,
                    sequence: lease.sequence,
                    topology_generation: generation,
                    events,
                };
                if !self.send(peer, &context, message) {
                    self.release(session_id, "input send queue failed", true)
                }
            }
            RuntimeCommand::LeaveInputControl { session_id } => self.leave(session_id),
            RuntimeCommand::ReleaseInputControl { session_id } => {
                self.release(session_id, "input control released", true)
            }
            _ => {}
        }
        self.tick();
    }
    fn leave(&mut self, id: InputSessionId) {
        self.flush_motion_snapshot(id);
        let Some(lease) = self.leases.get_mut(&id) else {
            return;
        };
        if !lease.controller || lease.token.is_none() {
            return;
        }
        lease.sequence = lease.sequence.saturating_add(1);
        lease.entered = false;
        lease.hybrid.dirty_at = None;
        lease.hybrid.receiver.leave();
        self.transport
            .cancel_input_motion(lease.peer, lease.context.session_id, id);
        let peer = lease.peer;
        let context = lease.context.clone();
        let message = InputShareMessage::Leave {
            session_id: id,
            token: lease.token.unwrap(),
            sequence: lease.sequence,
        };
        if !self.send(peer, &context, message) {
            self.release(id, "input leave failed", true)
        } else {
            self.emit_release(id, peer, "input left display")
        }
    }
    fn inbound(&mut self, event: InputTransportEvent) {
        match event {
            InputTransportEvent::Motion {
                peer_id,
                context,
                motion,
            } => self.inbound_motion(peer_id, context, motion),
            InputTransportEvent::Invalidated {
                peer_id,
                session_id,
                reason,
            } => {
                self.activation_invalidated(peer_id, session_id, &reason);
                let ids = self
                    .leases
                    .iter()
                    .filter(|(_, l)| l.peer == peer_id && l.context.session_id == session_id)
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>();
                for id in ids {
                    self.release(id, &reason, false)
                }
                self.emit(RuntimeEvent::InputLinkUnavailable { peer_id, reason });
            }
            InputTransportEvent::Message {
                peer_id,
                context,
                message,
            } => {
                if message.validate().is_err()
                    || !self.network_allowed(peer_id)
                    || !self.context_live(peer_id, &context)
                {
                    return;
                }
                let hybrid_request = matches!(&message, InputShareMessage::RequestHybrid { .. });
                let hybrid_grant = match &message {
                    InputShareMessage::GrantHybrid { motion_enabled, .. } => Some(*motion_enabled),
                    _ => None,
                };
                match message {
                    InputShareMessage::ForgetControllerEnvironment { ssid } => {
                        self.receive_forget_environment(peer_id, &context, ssid)
                    }
                    InputShareMessage::ControllerPresence {
                        controller,
                        ssid,
                        preferred,
                    } => self.receive_controller_presence(
                        peer_id, &context, controller, ssid, preferred,
                    ),
                    InputShareMessage::ControllerSelection {
                        controller,
                        ssid,
                        remember,
                    } => self.receive_controller_selection(
                        peer_id, &context, controller, ssid, remember,
                    ),
                    InputShareMessage::Hybrid {
                        session_id,
                        token,
                        sequence,
                        update,
                    } => self.inbound_hybrid(peer_id, context, session_id, token, sequence, update),
                    InputShareMessage::Activate {
                        activation_id,
                        intent_id,
                        ttl_ms: _,
                        sequence,
                    } => {
                        self.receive_activate(peer_id, context, activation_id, intent_id, sequence);
                    }
                    InputShareMessage::Activated {
                        activation_id,
                        ttl_ms: _,
                        sequence,
                        status,
                        reason,
                    } => {
                        self.receive_activated(
                            peer_id,
                            context,
                            activation_id,
                            sequence,
                            status,
                            reason,
                        );
                    }
                    InputShareMessage::Deactivate {
                        activation_id,
                        sequence,
                    } => {
                        self.receive_deactivate(peer_id, context, activation_id, sequence);
                    }
                    InputShareMessage::DisplayTopology(topology) => {
                        if self
                            .snapshot
                            .remote_topologies
                            .get(&peer_id)
                            .is_some_and(|current| {
                                topology.generation < current.generation
                                    || (topology.generation == current.generation
                                        && topology != *current)
                            })
                        {
                            return;
                        }
                        let changed =
                            self.snapshot.remote_topologies.get(&peer_id) != Some(&topology);
                        if changed {
                            let ids = self
                                .leases
                                .iter()
                                .filter(|(_, l)| {
                                    l.peer == peer_id
                                        && self.snapshot.remote_topologies.contains_key(&peer_id)
                                })
                                .map(|(id, _)| *id)
                                .collect::<Vec<_>>();
                            for id in ids {
                                self.release(id, "remote display topology changed", true)
                            }
                            self.snapshot
                                .remote_topologies
                                .insert(peer_id, topology.clone());
                            self.emit(RuntimeEvent::RemoteDisplayTopology { peer_id, topology });
                        }
                        // Display metadata is needed to configure sharing before native input
                        // permissions or control are enabled. The outer check still requires a
                        // trusted current member on an authenticated physical LAN context.
                        self.publish_topology(peer_id, &context, Instant::now());
                    }
                    InputShareMessage::Request {
                        session_id,
                        target_display,
                    }
                    | InputShareMessage::RequestHybrid {
                        session_id,
                        target_display,
                    } => {
                        if hybrid_request
                            && !self
                                .transport
                                .input_capabilities(peer_id, context.session_id)
                                .contains(Capabilities::INPUT_MOTION_DATAGRAM_V1)
                        {
                            return;
                        }
                        let allowed =
                            self.role_allowed(peer_id, false)
                                && self
                                    .incoming
                                    .as_ref()
                                    .is_some_and(|a| a.context.as_ref() == Some(&context))
                                && self.snapshot.local_topology.as_ref().is_some_and(|t| {
                                    t.displays.iter().any(|d| d.id == target_display)
                                });
                        let conflict = self
                            .leases
                            .iter()
                            .any(|(id, l)| *id != session_id && !l.controller && !l.expired());
                        if !allowed || conflict {
                            self.send(
                                peer_id,
                                &context,
                                InputShareMessage::Denied {
                                    session_id,
                                    reason: if conflict {
                                        "Another controller owns input"
                                    } else {
                                        "This controller is not authorized or input is unavailable"
                                    }
                                    .into(),
                                },
                            );
                            return;
                        }
                        if self.leases.contains_key(&session_id) {
                            return;
                        }
                        let token = InputToken::new();
                        let revision = self.revision();
                        self.leases.insert(
                            session_id,
                            Lease {
                                activation_id: self.incoming.as_ref().unwrap().id,
                                peer: peer_id,
                                context: context.clone(),
                                token: Some(token),
                                controller: false,
                                target: target_display,
                                entered: false,
                                sequence: 0,
                                deadline: Instant::now() + TTL,
                                continuous_deadline_ms: lease_now_ms()
                                    .checked_add(1500)
                                    .unwrap_or(0),
                                renew_at: Instant::now() + RENEW,
                                awaiting_ack: false,
                                revision,
                                hybrid: HybridLease {
                                    requested: hybrid_request,
                                    fallback_reason: if !hybrid_request {
                                        Some("Controller requested reliable-only".into())
                                    } else if !self
                                        .transport
                                        .input_motion_available(peer_id, context.session_id)
                                    {
                                        Some("Receiver Datagram path unavailable".into())
                                    } else {
                                        None
                                    },
                                    enabled: hybrid_request
                                        && self
                                            .transport
                                            .input_motion_available(peer_id, context.session_id),
                                    ..Default::default()
                                },
                            },
                        );
                        if !self.send(
                            peer_id,
                            &context,
                            if hybrid_request {
                                InputShareMessage::GrantHybrid {
                                    session_id,
                                    token,
                                    ttl_ms: 1500,
                                    motion_enabled: self.leases[&session_id].hybrid.enabled,
                                }
                            } else {
                                InputShareMessage::Grant {
                                    session_id,
                                    token,
                                    ttl_ms: 1500,
                                }
                            },
                        ) {
                            self.release(session_id, "input grant send failed", false)
                        }
                    }
                    InputShareMessage::Grant {
                        session_id, token, ..
                    }
                    | InputShareMessage::GrantHybrid {
                        session_id, token, ..
                    } => {
                        let valid = self.leases.get(&session_id).is_some_and(|l| {
                            l.peer == peer_id
                                && l.context == context
                                && l.controller
                                && l.awaiting_ack
                                && !l.expired()
                                && l.token.is_none_or(|t| t == token)
                                && l.hybrid.requested == hybrid_grant.is_some()
                                && (l.token.is_none()
                                    || l.hybrid.enabled == hybrid_grant.unwrap_or(false))
                                && (!hybrid_grant.unwrap_or(false)
                                    || self
                                        .transport
                                        .input_motion_available(peer_id, context.session_id))
                        }) && self.role_allowed(peer_id, true);
                        if !valid {
                            return;
                        }
                        let revision = self.revision();
                        let lease = self.leases.get_mut(&session_id).unwrap();
                        let first = lease.token.is_none();
                        if first {
                            lease.hybrid.enabled = hybrid_grant.unwrap_or(false);
                            if hybrid_grant == Some(false) {
                                lease.hybrid.fallback_reason =
                                    Some("Peer selected reliable-only".into());
                            }
                        }
                        lease.token = Some(token);
                        lease.extend();
                        lease.awaiting_ack = false;
                        lease.revision = revision;
                        if first {
                            self.emit(RuntimeEvent::InputControlGranted {
                                peer_id,
                                session_id,
                                lease_expires_at: self.now_ms() + 1500,
                            });
                        }
                    }
                    InputShareMessage::Denied { session_id, reason } => {
                        if self.leases.get(&session_id).is_some_and(|l| {
                            l.peer == peer_id
                                && l.context == context
                                && l.controller
                                && l.token.is_none()
                        }) {
                            self.leases.remove(&session_id);
                            self.attempted.insert(peer_id, Instant::now());
                            self.emit(RuntimeEvent::InputControlDenied {
                                peer_id,
                                session_id,
                                reason,
                            });
                        }
                    }
                    authenticated => self.authenticated(peer_id, context, authenticated),
                }
            }
        }
    }
    fn authenticated(
        &mut self,
        peer: DeviceId,
        context: ConnectionContext,
        message: InputShareMessage,
    ) {
        let (id, token, sequence) = match &message {
            InputShareMessage::Enter {
                session_id,
                token,
                sequence,
                ..
            }
            | InputShareMessage::Frame {
                session_id,
                token,
                sequence,
                ..
            }
            | InputShareMessage::Renew {
                session_id,
                token,
                sequence,
            }
            | InputShareMessage::Leave {
                session_id,
                token,
                sequence,
            }
            | InputShareMessage::Release {
                session_id,
                token,
                sequence,
            } => (*session_id, *token, *sequence),
            _ => return,
        };
        let Some(lease) = self.leases.get(&id) else {
            return;
        };
        if lease.peer != peer || lease.context != context || lease.token != Some(token) {
            return;
        }
        if lease.expired()
            || sequence <= lease.sequence
            || !self.role_allowed(peer, lease.controller)
            || !self.lease_activation_live(lease)
        {
            self.release(id, "input lease expired or replayed", true);
            return;
        }
        // Release is valid in either direction. Other state-changing messages originate at the controller.
        if lease.controller && !matches!(message, InputShareMessage::Release { .. }) {
            return;
        }
        if matches!(message, InputShareMessage::Release { .. }) {
            self.release(id, "peer released input control", false);
            return;
        }
        if lease.hybrid.enabled
            && matches!(
                &message,
                InputShareMessage::Enter { .. } | InputShareMessage::Frame { .. }
            )
        {
            self.release(id, "legacy frame in hybrid session", true);
            return;
        }
        self.leases.get_mut(&id).unwrap().sequence = sequence;
        match message {
            InputShareMessage::Renew { .. } => {
                let revision = self.revision();
                let lease = self.leases.get_mut(&id).unwrap();
                lease.extend();
                lease.revision = revision;
                let ack = if lease.hybrid.requested {
                    InputShareMessage::GrantHybrid {
                        session_id: id,
                        token,
                        ttl_ms: 1500,
                        motion_enabled: lease.hybrid.enabled,
                    }
                } else {
                    InputShareMessage::Grant {
                        session_id: id,
                        token,
                        ttl_ms: 1500,
                    }
                };
                if !self.send(peer, &context, ack) {
                    self.release(id, "input renewal acknowledgement failed", false)
                }
            }
            InputShareMessage::Leave { .. } => {
                let lease = self.leases.get_mut(&id).unwrap();
                lease.entered = false;
                lease.hybrid.receiver.leave();
                self.emit_release(id, peer, "peer left display");
            }
            InputShareMessage::Enter {
                target_display,
                x,
                y,
                ..
            } => {
                if !self.snapshot.local_topology.as_ref().is_some_and(|t| {
                    t.displays
                        .iter()
                        .any(|d| d.id == target_display && d.contains(x, y))
                }) {
                    self.release(id, "invalid input entry target", true);
                    return;
                }
                let lease = self.leases.get_mut(&id).unwrap();
                lease.entered = true;
                lease.target = target_display.clone();
                self.emit(RuntimeEvent::SharedInputReceived {
                    peer_id: peer,
                    session_id: id,
                    target_display,
                    sequence,
                    state_epoch: 0,
                    events: vec![SharedInputEvent::Motion { x, y }],
                });
            }
            InputShareMessage::Frame {
                target_display,
                topology_generation,
                events,
                ..
            } => {
                let lease = self.leases.get(&id).unwrap();
                let valid = lease.entered
                    && lease.target == target_display
                    && self.snapshot.local_topology.as_ref().is_some_and(|t| {
                        t.generation == topology_generation
                            && t.displays.iter().any(|d| {
                                d.id == target_display
                                    && events.iter().all(|e| match e {
                                        SharedInputEvent::Motion { x, y } => d.contains(*x, *y),
                                        _ => true,
                                    })
                            })
                    });
                if !valid {
                    self.release(id, "input frame target or topology invalid", true);
                    return;
                }
                self.emit(RuntimeEvent::SharedInputReceived {
                    peer_id: peer,
                    session_id: id,
                    target_display,
                    sequence,
                    state_epoch: 0,
                    events,
                });
            }
            _ => {}
        }
    }
    fn tick(&mut self) {
        self.tick_links();
        let mut membership = self.network.borrow().device_network.clone();
        membership.pending.clear();
        if self
            .membership
            .as_ref()
            .is_some_and(|old| old != &membership)
        {
            self.release_all("device network membership changed");
        }
        self.membership = Some(membership);
        self.tick_environment();
        self.tick_activations();
        let now = Instant::now();
        self.motion_disabled_until.retain(|_, until| *until > now);
        self.motion_probe_started
            .retain(|_, start| now.saturating_duration_since(*start) < Duration::from_secs(60));
        let disconnected = self
            .snapshot
            .remote_topologies
            .keys()
            .copied()
            .filter(|peer| !self.network_allowed(*peer))
            .collect::<Vec<_>>();
        for peer in disconnected {
            self.snapshot.remote_topologies.remove(&peer);
            self.published_topologies
                .retain(|(id, _, _), _| *id != peer);
        }
        let expired = self
            .leases
            .iter()
            .filter(|(_, l)| {
                l.expired()
                    || !self.network_allowed(l.peer)
                    || !self.role_allowed(l.peer, l.controller)
                    || !self.context_live(l.peer, &l.context)
                    || !self.lease_activation_live(l)
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in expired {
            self.release(id, "input lease expired or eligibility changed", true)
        }
        self.tick_motion();
        let renew = self
            .leases
            .iter()
            .filter(|(_, l)| l.controller && l.token.is_some() && now >= l.renew_at)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in renew {
            let lease = self.leases.get_mut(&id).unwrap();
            lease.sequence = lease.sequence.saturating_add(1);
            lease.renew_at = now + RENEW;
            lease.awaiting_ack = true;
            let peer = lease.peer;
            let context = lease.context.clone();
            let message = InputShareMessage::Renew {
                session_id: id,
                token: lease.token.unwrap(),
                sequence: lease.sequence,
            };
            if !self.send(peer, &context, message) {
                self.release(id, "input renew send failed", true)
            }
        }
        let peers = self.network.borrow().snapshot.peers.clone();
        for peer in peers {
            let id = peer.id;
            if !self.network_allowed(id) {
                continue;
            }
            let contexts = self.transport.input_contexts(id);
            let Some(context) = contexts.into_iter().find(ConnectionContext::input_eligible) else {
                self.start_link(
                    id,
                    peer.routes
                        .iter()
                        .filter(|r| !r.source.is_public())
                        .map(|r| r.endpoint.clone())
                        .collect(),
                );
                continue;
            };
            self.publish_topology(id, &context, now);
            if self.role_allowed(id, true)
                && !self.leases.values().any(|l| l.peer == id)
                && self
                    .attempted
                    .get(&id)
                    .is_none_or(|t| now.duration_since(*t) >= Duration::from_secs(2))
                && let Some(target) = self
                    .snapshot
                    .remote_topologies
                    .get(&id)
                    .and_then(|t| {
                        t.displays
                            .iter()
                            .find(|d| d.is_primary)
                            .or_else(|| t.displays.first())
                    })
                    .map(|d| d.id.clone())
            {
                self.attempted.insert(id, now);
                self.request(id, target);
            }
        }
    }
    fn publish(&mut self) {
        self.publish_activations();
        let effective = self.snapshot.effective_settings();
        let network = self.network.borrow().clone();
        self.snapshot.links = network
            .snapshot
            .peers
            .iter()
            .map(|peer| {
                let lease = self
                    .leases
                    .iter()
                    .find(|(_, l)| l.peer == peer.id && l.token.is_some());
                let context = lease.map(|(_, l)| l.context.clone()).or_else(|| {
                    self.transport
                        .input_contexts(peer.id)
                        .into_iter()
                        .find(ConnectionContext::input_eligible)
                });
                let ready = lease.is_some_and(|(_, l)| {
                    !l.expired()
                        && self.role_allowed(peer.id, l.controller)
                        && self.network_allowed(peer.id)
                        && self.context_live(peer.id, &l.context)
                });
                let reason = if !self.available {
                    self.unavailable_reason
                        .clone()
                        .or_else(|| Some("Input permissions unavailable".into()))
                } else if !effective.enabled {
                    Some("Input sharing disabled".into())
                } else if !self.network_allowed(peer.id) {
                    Some("Requires a trusted connected peer with input sharing capability".into())
                } else if context.is_none() {
                    Some("No verified physical on-link LAN connection".into())
                } else if !ready {
                    Some("Awaiting authorized input lease".into())
                } else {
                    None
                };
                InputPeerStatus {
                    peer_id: peer.id,
                    context,
                    ready,
                    reason,
                    control_session: lease.map(|(id, _)| *id),
                    lease_revision: lease.map_or(0, |(_, l)| l.revision),
                    motion_fallback_reason: lease
                        .and_then(|(_, l)| l.hybrid.fallback_reason.clone()),
                    motion_mode: lease.map(|(_, l)| {
                        if l.hybrid.enabled {
                            "hybrid"
                        } else {
                            "reliable-only"
                        }
                        .to_owned()
                    }),
                }
            })
            .collect();
        self.output.send_if_modified(|current| {
            if *current == self.snapshot {
                false
            } else {
                *current = self.snapshot.clone();
                true
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MemoryRuntimeStore, ProviderFuture, TransportConnection};
    use std::sync::Mutex;
    #[derive(Default)]
    pub(super) struct TestTransport {
        pub(super) contexts: Mutex<Vec<ConnectionContext>>,
        pub(super) sent: Mutex<Vec<(DeviceId, TransportSessionId, InputShareMessage)>>,
        pub(super) capabilities: Mutex<BTreeMap<TransportSessionId, Capabilities>>,
        ensured: Mutex<Vec<DeviceId>>,
        pub(super) motion_ready: AtomicBool,
        pub(super) motion_sent: Mutex<Vec<MotionDatagram>>,
        pub(super) motion_cancelled: Mutex<Vec<InputSessionId>>,
    }
    impl TransportProvider for TestTransport {
        fn connect(
            &self,
            _hello: unidesk_protocol::Hello,
            _peer: DeviceId,
            _route: RouteCandidate,
        ) -> ProviderFuture<TransportConnection> {
            Box::pin(async { Err(RuntimeError::ChannelClosed) })
        }
        fn disconnect(&self, _peer: DeviceId) -> ProviderFuture<()> {
            Box::pin(async { Ok(()) })
        }
        fn input_contexts(&self, _peer: DeviceId) -> Vec<ConnectionContext> {
            self.contexts.lock().unwrap().clone()
        }
        fn input_capabilities(&self, _peer: DeviceId, session: TransportSessionId) -> Capabilities {
            if let Some(capabilities) = self.capabilities.lock().unwrap().get(&session) {
                return *capabilities;
            }
            if self
                .contexts
                .lock()
                .unwrap()
                .iter()
                .any(|context| context.session_id == session)
            {
                Capabilities::INPUT_SHARING | Capabilities::INPUT_ACTIVATION
            } else {
                Capabilities::default()
            }
        }
        fn input_motion_available(&self, _peer: DeviceId, _session: TransportSessionId) -> bool {
            self.motion_ready.load(Ordering::Acquire)
        }
        fn send_input_motion(
            &self,
            _peer: DeviceId,
            _session: TransportSessionId,
            motion: MotionDatagram,
        ) -> std::result::Result<(), RuntimeError> {
            self.motion_sent.lock().unwrap().push(motion);
            Ok(())
        }
        fn cancel_input_motion(
            &self,
            _peer: DeviceId,
            _session: TransportSessionId,
            control: InputSessionId,
        ) {
            self.motion_cancelled.lock().unwrap().push(control);
        }
        fn ensure_input_link(
            &self,
            peer: DeviceId,
            _endpoints: Vec<Endpoint>,
        ) -> ProviderFuture<()> {
            self.ensured.lock().unwrap().push(peer);
            Box::pin(async { Ok(()) })
        }
        fn send_input(
            &self,
            peer: DeviceId,
            session: TransportSessionId,
            message: InputShareMessage,
        ) -> std::result::Result<(), RuntimeError> {
            self.sent.lock().unwrap().push((peer, session, message));
            Ok(())
        }
    }
    pub(super) fn setup(
        controller: bool,
    ) -> (
        InputActor,
        DeviceId,
        ConnectionContext,
        Arc<TestTransport>,
        Arc<InputDelivery>,
    ) {
        let peer = DeviceId::from_public_key([9; 32]).unwrap();
        let context = ConnectionContext {
            session_id: TransportSessionId::new(),
            local_addr: "10.0.0.1:1111".parse().unwrap(),
            remote_addr: "10.0.0.2:2222".parse().unwrap(),
            interface_name: Some("en0".into()),
            interface_kind: InterfaceKind::WiFi,
            scope: LinkScope::Lan,
        };
        let transport = Arc::new(TestTransport::default());
        transport.contexts.lock().unwrap().push(context.clone());
        let route = RouteCandidate::new(
            Endpoint::new("10.0.0.2", 2222).unwrap(),
            RouteSource::Mdns,
            RouteHealth::Healthy,
            Capabilities::INPUT_SHARING,
        );
        let peer_snapshot = PeerSnapshot::new(
            peer,
            "remote",
            [9; 32],
            LivenessState::Online,
            TrustState::Trusted,
            ConnectionState::Connected,
            vec![route.clone()],
            Some(route.id),
        )
        .unwrap();
        let network = NetworkSnapshot::new(
            0,
            DeviceId::from_public_key([8; 32]).unwrap(),
            vec![peer_snapshot],
            false,
        )
        .unwrap();
        let (_, network) = watch::channel(crate::SnapshotView {
            snapshot: network,
            discovered_peers: vec![],
            device_network: Default::default(),
        });
        let topology = DisplayTopology {
            generation: 7,
            displays: vec![DisplayDescriptor {
                id: DisplayId::from("display"),
                frame_x: 0.,
                frame_y: 0.,
                width: 1000,
                height: 800,
                scale_factor: 2.,
                rotation_degrees: 0,
                is_primary: true,
            }],
        };
        let settings = InputSharingSettings {
            enabled: controller,
            role: if controller {
                InputShareRole::Controller
            } else {
                InputShareRole::Disabled
            },
            participants: if controller { vec![peer] } else { vec![] },
            authorized_controller: None,
            ..Default::default()
        };
        let snapshot = InputSharingSnapshot {
            environment: Default::default(),
            managed_control: None,
            activations: BTreeMap::new(),
            scenes: Vec::new(),
            settings,
            local_topology: Some(topology.clone()),
            remote_topologies: BTreeMap::from([(peer, topology)]),
            links: vec![],
        };
        let (output, _) = watch::channel(snapshot.clone());
        let (events, _) = broadcast::channel(128);
        let delivery = Arc::new(InputDelivery::new(Arc::new(AtomicBool::new(false))));
        let receiver = delivery.clone();
        let activation = Activation::ready_fixture(peer, context.clone());
        (
            InputActor {
                links: recovery::Links::default(),
                environment: environment::Environment::default(),
                outgoing: if controller {
                    BTreeMap::from([(peer, Activation::ready_fixture(peer, context.clone()))])
                } else {
                    BTreeMap::new()
                },
                incoming: if controller {
                    None
                } else {
                    Some(activation.clone())
                },
                retired_activations: std::collections::VecDeque::new(),
                paused_intents: std::collections::BTreeSet::new(),
                controller_intents: if controller {
                    BTreeMap::from([(peer, activation.intent_id)])
                } else {
                    BTreeMap::new()
                },
                activation_policy_error: None,
                pending_policy_save: None,
                recent_managed_intent: if controller {
                    None
                } else {
                    Some((peer, activation.intent_id))
                },
                snapshot,
                network,
                membership: None,
                transport: transport.clone(),
                store: Arc::new(MemoryRuntimeStore::default()),
                events,
                delivery,
                output,
                available: true,
                unavailable_reason: None,
                leases: BTreeMap::new(),
                published_topologies: BTreeMap::new(),
                attempted: BTreeMap::new(),
                motion_probe_started: BTreeMap::new(),
                motion_disabled_until: BTreeMap::new(),
                started: Instant::now(),
                revision: 0,
            },
            peer,
            context,
            transport,
            receiver,
        )
    }
    pub(super) fn receive(
        actor: &mut InputActor,
        peer: DeviceId,
        context: &ConnectionContext,
        message: InputShareMessage,
    ) {
        actor.inbound(InputTransportEvent::Message {
            peer_id: peer,
            context: context.clone(),
            message,
        })
    }
    pub(super) fn receiver_grant(
        actor: &mut InputActor,
        peer: DeviceId,
        context: &ConnectionContext,
    ) -> (InputSessionId, InputToken) {
        let id = InputSessionId::new();
        receive(
            actor,
            peer,
            context,
            InputShareMessage::Request {
                session_id: id,
                target_display: "display".into(),
            },
        );
        (id, actor.leases[&id].token.unwrap())
    }
    fn motion_command(session_id: InputSessionId, x: f64) -> RuntimeCommand {
        RuntimeCommand::SendSharedInput {
            session_id,
            target_display: "display".into(),
            events: vec![SharedInputEvent::Motion { x, y: 10.0 }],
        }
    }

    #[test]
    fn queued_motion_keeps_latest_position_and_stops_at_ordering_boundaries() {
        let (mut actor, peer, _, _, _) = setup(true);
        actor.request(peer, "display".into());
        let id = *actor.leases.keys().next().unwrap();
        let mut barriers = vec![
            RuntimeCommand::LeaveInputControl { session_id: id },
            RuntimeCommand::ReleaseInputControl { session_id: id },
            RuntimeCommand::SetInputAvailability {
                available: false,
                reason: None,
            },
            motion_command(InputSessionId::new(), 30.0),
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
                code: 0,
                pressed: true,
                modifiers: 0,
                repeat: false,
            },
            SharedInputEvent::Scroll { dx: 0.0, dy: 1.0 },
            SharedInputEvent::Modifiers { modifiers: 1 },
        ] {
            barriers.push(RuntimeCommand::SendSharedInput {
                session_id: id,
                target_display: "display".into(),
                events: vec![event],
            });
        }
        barriers.push(RuntimeCommand::SendSharedInput {
            session_id: id,
            target_display: "another-display".into(),
            events: vec![SharedInputEvent::Motion { x: 30.0, y: 10.0 }],
        });
        for barrier in barriers {
            let (tx, mut rx) = mpsc::channel(8);
            tx.try_send(motion_command(id, 20.0)).unwrap();
            tx.try_send(barrier.clone()).unwrap();
            tx.try_send(motion_command(id, 40.0)).unwrap();
            let mut pending = None;
            let command =
                actor.coalesce_queued_motion(motion_command(id, 10.0), &mut rx, &mut pending);
            assert_eq!(command, motion_command(id, 20.0));
            assert_eq!(pending, Some(barrier));
            assert_eq!(rx.try_recv().unwrap(), motion_command(id, 40.0));
        }
    }

    #[test]
    fn queued_motion_does_not_hide_invalid_or_out_of_display_frames() {
        let (mut actor, peer, _, _, _) = setup(true);
        actor.request(peer, "display".into());
        let id = *actor.leases.keys().next().unwrap();
        for events in [
            vec![],
            vec![SharedInputEvent::Motion { x: 1.0, y: 1.0 }; 257],
            vec![SharedInputEvent::Motion {
                x: f64::NAN,
                y: 1.0,
            }],
            vec![SharedInputEvent::Motion { x: 1500.0, y: 1.0 }],
            vec![
                SharedInputEvent::Motion { x: -1.0, y: 1.0 },
                SharedInputEvent::Motion { x: 1.0, y: 1.0 },
            ],
        ] {
            let invalid = RuntimeCommand::SendSharedInput {
                session_id: id,
                target_display: "display".into(),
                events,
            };
            let (tx, mut rx) = mpsc::channel(8);
            tx.try_send(invalid).unwrap();
            tx.try_send(motion_command(id, 40.0)).unwrap();
            let mut pending = None;
            let command =
                actor.coalesce_queued_motion(motion_command(id, 10.0), &mut rx, &mut pending);
            assert_eq!(command, motion_command(id, 10.0));
            let invalid = pending
                .take()
                .expect("invalid frame must remain an ordering barrier");
            assert!(actor.motion_command_target(&invalid).is_none());
            let unchanged = actor.coalesce_queued_motion(invalid, &mut rx, &mut pending);
            assert!(actor.motion_command_target(&unchanged).is_none());
            assert!(pending.is_none());
            assert_eq!(rx.try_recv().unwrap(), motion_command(id, 40.0));
        }
    }

    #[test]
    fn queued_motion_drain_is_bounded_and_compacts_absolute_frames() {
        let (mut actor, peer, _, _, _) = setup(true);
        actor.request(peer, "display".into());
        let id = *actor.leases.keys().next().unwrap();
        let (tx, mut rx) = mpsc::channel(128);
        for i in 1..=MAX_MOTION_COMMAND_DRAIN {
            tx.try_send(motion_command(id, i as f64)).unwrap();
        }
        let mut pending = None;
        let latest = actor.coalesce_queued_motion(motion_command(id, 0.0), &mut rx, &mut pending);
        assert_eq!(
            latest,
            motion_command(id, (MAX_MOTION_COMMAND_DRAIN - 1) as f64)
        );
        assert_eq!(
            rx.try_recv().unwrap(),
            motion_command(id, MAX_MOTION_COMMAND_DRAIN as f64)
        );
        assert!(pending.is_none());
        let batch = RuntimeCommand::SendSharedInput {
            session_id: id,
            target_display: "display".into(),
            events: vec![
                SharedInputEvent::Motion { x: 1.0, y: 2.0 },
                SharedInputEvent::Motion { x: 3.0, y: 10.0 },
            ],
        };
        assert_eq!(
            actor.coalesce_queued_motion(batch, &mut rx, &mut pending),
            motion_command(id, 3.0)
        );
    }

    #[test]
    fn receiver_requires_enter_and_releases_on_replay() {
        let (mut actor, peer, context, _, events) = setup(false);
        let (id, token) = receiver_grant(&mut actor, peer, &context);
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Frame {
                session_id: id,
                token,
                target_display: "display".into(),
                sequence: 1,
                topology_generation: 7,
                events: vec![SharedInputEvent::Button {
                    button: 0,
                    pressed: true,
                }],
            },
        );
        assert!(!actor.leases.contains_key(&id));
        assert!(
            !std::iter::from_fn(|| events.try_recv().ok())
                .any(|e| matches!(e, RuntimeEvent::SharedInputReceived { .. }))
        );
        let (id, token) = receiver_grant(&mut actor, peer, &context);
        let enter = InputShareMessage::Enter {
            session_id: id,
            token,
            target_display: "display".into(),
            x: 2.,
            y: 3.,
            sequence: 1,
        };
        receive(&mut actor, peer, &context, enter.clone());
        assert!(actor.leases[&id].entered);
        receive(&mut actor, peer, &context, enter);
        assert!(!actor.leases.contains_key(&id));
    }
    #[test]
    fn only_exact_controller_and_lan_can_get_grant() {
        let (mut actor, peer, context, transport, _) = setup(false);
        actor.incoming.as_mut().unwrap().peer = DeviceId::from_bytes([0; 32]);
        let id = InputSessionId::new();
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Request {
                session_id: id,
                target_display: "display".into(),
            },
        );
        assert!(actor.leases.is_empty());
        assert!(matches!(
            transport.sent.lock().unwrap().last().unwrap().2,
            InputShareMessage::Denied { .. }
        ));
        actor.incoming.as_mut().unwrap().peer = peer;
        let mut vpn = context.clone();
        vpn.scope = LinkScope::Overlay;
        vpn.interface_kind = InterfaceKind::Vpn;
        *transport.contexts.lock().unwrap() = vec![vpn.clone()];
        receive(
            &mut actor,
            peer,
            &vpn,
            InputShareMessage::Request {
                session_id: id,
                target_display: "display".into(),
            },
        );
        assert!(actor.leases.is_empty());
    }
    #[test]
    fn second_controller_session_conflicts_and_lease_expires() {
        let (mut actor, peer, context, _, _) = setup(false);
        let (id, _) = receiver_grant(&mut actor, peer, &context);
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Request {
                session_id: InputSessionId::new(),
                target_display: "display".into(),
            },
        );
        assert_eq!(actor.leases.len(), 1);
        actor.leases.get_mut(&id).unwrap().deadline = Instant::now() - Duration::from_millis(1);
        actor.tick();
        assert!(actor.leases.is_empty());
    }
    #[test]
    fn ack_preserves_sequence_and_entry_and_cannot_resurrect_release() {
        let (mut actor, peer, context, _, events) = setup(true);
        actor.request(peer, "display".into());
        let id = *actor.leases.keys().next().unwrap();
        let token = InputToken::new();
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Grant {
                session_id: id,
                token,
                ttl_ms: 1500,
            },
        );
        let initial_revision = actor.leases[&id].revision;
        let lease = actor.leases.get_mut(&id).unwrap();
        lease.sequence = 8;
        lease.entered = true;
        lease.awaiting_ack = true;
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Grant {
                session_id: id,
                token,
                ttl_ms: 1500,
            },
        );
        assert_eq!(actor.leases[&id].sequence, 8);
        assert!(actor.leases[&id].entered);
        assert!(actor.leases[&id].revision > initial_revision);
        assert_eq!(
            std::iter::from_fn(|| events.try_recv().ok())
                .filter(|e| matches!(e, RuntimeEvent::InputControlGranted { .. }))
                .count(),
            1
        );
        actor.release(id, "test release", true);
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Grant {
                session_id: id,
                token,
                ttl_ms: 1500,
            },
        );
        assert!(actor.leases.is_empty());
    }
    #[test]
    fn same_peer_new_transport_cannot_grant_old_pending_session() {
        let (mut actor, peer, context, transport, _) = setup(true);
        actor.request(peer, "display".into());
        let id = *actor.leases.keys().next().unwrap();
        let mut another = context.clone();
        another.session_id = TransportSessionId::new();
        transport.contexts.lock().unwrap().push(another.clone());
        receive(
            &mut actor,
            peer,
            &another,
            InputShareMessage::Grant {
                session_id: id,
                token: InputToken::new(),
                ttl_ms: 1500,
            },
        );
        assert!(actor.leases[&id].token.is_none());
    }
    #[test]
    fn renew_ack_required_and_sender_success_does_not_extend_lease() {
        let (mut actor, peer, context, _, _) = setup(true);
        actor.request(peer, "display".into());
        let id = *actor.leases.keys().next().unwrap();
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Grant {
                session_id: id,
                token: InputToken::new(),
                ttl_ms: 1500,
            },
        );
        let deadline = actor.leases[&id].deadline;
        actor.leases.get_mut(&id).unwrap().renew_at = Instant::now() - Duration::from_millis(1);
        actor.tick();
        assert_eq!(actor.leases[&id].deadline, deadline);
        assert!(actor.leases[&id].awaiting_ack);
        actor.leases.get_mut(&id).unwrap().deadline = Instant::now() - Duration::from_millis(1);
        actor.tick();
        assert!(!actor.leases.contains_key(&id));
    }
    #[test]
    fn unchanged_snapshot_does_not_renew_native_watchdog() {
        let (mut actor, peer, context, _, _) = setup(false);
        let (id, _) = receiver_grant(&mut actor, peer, &context);
        actor.publish();
        let revision = actor.snapshot.links[0].lease_revision;
        actor.publish();
        assert_eq!(actor.snapshot.links[0].lease_revision, revision);
        assert_eq!(actor.snapshot.links[0].control_session, Some(id));
    }
    #[tokio::test]
    async fn loss_of_context_releases_without_waiting_for_discovery() {
        let (mut actor, peer, context, transport, _) = setup(false);
        let (id, _) = receiver_grant(&mut actor, peer, &context);
        transport.contexts.lock().unwrap().clear();
        actor.tick();
        assert!(!actor.leases.contains_key(&id));
    }
    #[test]
    fn continuous_clock_expiry_rejects_renew_after_system_sleep() {
        let (mut actor, peer, context, _, _) = setup(false);
        let (id, token) = receiver_grant(&mut actor, peer, &context);
        let lease = actor.leases.get_mut(&id).unwrap();
        lease.deadline = Instant::now() + TTL;
        lease.continuous_deadline_ms = lease_now_ms().saturating_sub(1);
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Renew {
                session_id: id,
                token,
                sequence: 1,
            },
        );
        assert!(!actor.leases.contains_key(&id));
    }
    #[tokio::test]
    async fn scene_apply_reclaims_and_current_membership_cannot_be_bypassed() {
        let (mut actor, peer, context, _, _) = setup(false);
        actor
            .command(RuntimeCommand::SaveInputSharingScene {
                name: "Desk".into(),
            })
            .await;
        let scene = actor.snapshot.scenes[0].clone();
        receiver_grant(&mut actor, peer, &context);
        actor
            .command(RuntimeCommand::ApplyInputSharingScene {
                scene_id: scene.id.clone(),
            })
            .await;
        assert!(actor.leases.is_empty());
        assert_eq!(actor.snapshot.settings, scene.settings);
        let mut view = actor.network.borrow().clone();
        let local = view.snapshot.local_device_id;
        view.device_network.network_id = Some("test network".into());
        view.device_network.member_ids = vec![local];
        let (_, network) = watch::channel(view);
        actor.network = network;
        actor
            .command(RuntimeCommand::ApplyInputSharingScene { scene_id: scene.id })
            .await;
        assert!(!actor.network_allowed(peer));
        let id = InputSessionId::new();
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Request {
                session_id: id,
                target_display: "display".into(),
            },
        );
        assert!(actor.leases.is_empty());
        let mut view = actor.network.borrow().clone();
        view.device_network.network_id = None;
        view.device_network.integrity_error = Some("bad signature".into());
        let (_, network) = watch::channel(view);
        actor.network = network;
        assert!(!actor.network_allowed(peer));
    }
    #[test]
    fn initial_controller_topology_arriving_after_enter_is_not_a_topology_change() {
        let (mut actor, peer, context, _, _) = setup(false);
        let topology = actor.snapshot.remote_topologies.remove(&peer).unwrap();
        let (id, token) = receiver_grant(&mut actor, peer, &context);
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Enter {
                session_id: id,
                token,
                target_display: "display".into(),
                x: 1.0,
                y: 2.0,
                sequence: 1,
            },
        );
        assert!(actor.leases[&id].entered);
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::DisplayTopology(topology.clone()),
        );
        assert!(actor.leases[&id].entered);
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::DisplayTopology(topology.clone()),
        );
        assert!(actor.leases[&id].entered);
        let mut changed = topology;
        changed.generation += 1;
        changed.displays[0].width += 1;
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::DisplayTopology(changed),
        );
        assert!(!actor.leases.contains_key(&id));
    }
    #[test]
    fn selected_peers_exchange_topology_without_enabling_input_or_native_permissions() {
        for (enabled, available, receiver_role) in [
            (false, false, InputShareRole::Disabled),
            (false, true, InputShareRole::Controlled),
        ] {
            let (mut controller, peer, context, transport, controller_events) = setup(true);
            let (mut receiver, receiver_peer, receiver_context, receiver_transport, events) =
                setup(false);
            for actor in [&mut controller, &mut receiver] {
                actor.incoming = None;
                actor.outgoing.clear();
                actor.snapshot.settings.enabled = enabled;
                actor.available = available;
                actor.snapshot.remote_topologies.clear();
            }
            receiver.snapshot.settings.role = receiver_role;
            receiver.snapshot.local_topology.as_mut().unwrap().displays[0].width = 1920;
            let controller_topology = controller.snapshot.local_topology.clone().unwrap();
            let receiver_topology = receiver.snapshot.local_topology.clone().unwrap();

            controller.tick();
            let messages = std::mem::take(&mut *transport.sent.lock().unwrap());
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].1, context.session_id);
            assert_eq!(
                messages[0].2,
                InputShareMessage::DisplayTopology(controller_topology.clone())
            );
            receive(
                &mut receiver,
                receiver_peer,
                &receiver_context,
                messages[0].2.clone(),
            );
            assert_eq!(
                receiver.snapshot.remote_topologies[&receiver_peer],
                controller_topology
            );
            let replies = std::mem::take(&mut *receiver_transport.sent.lock().unwrap());
            assert_eq!(replies.len(), 1);
            assert_eq!(replies[0].1, receiver_context.session_id);
            assert_eq!(
                replies[0].2,
                InputShareMessage::DisplayTopology(receiver_topology.clone())
            );
            receive(&mut controller, peer, &context, replies[0].2.clone());
            assert_eq!(
                controller.snapshot.remote_topologies[&peer],
                receiver_topology
            );
            // Replies share the periodic publication limit, so metadata cannot ping-pong.
            assert!(transport.sent.lock().unwrap().is_empty());

            controller.request(peer, "display".into());
            let session_id = InputSessionId::new();
            receive(
                &mut receiver,
                receiver_peer,
                &receiver_context,
                InputShareMessage::Request {
                    session_id,
                    target_display: "display".into(),
                },
            );
            assert!(matches!(
                receiver_transport.sent.lock().unwrap().last().unwrap().2,
                InputShareMessage::Denied { .. }
            ));
            let token = InputToken::new();
            receive(
                &mut receiver,
                receiver_peer,
                &receiver_context,
                InputShareMessage::Enter {
                    session_id,
                    token,
                    target_display: "display".into(),
                    x: 1.,
                    y: 2.,
                    sequence: 1,
                },
            );
            receive(
                &mut receiver,
                receiver_peer,
                &receiver_context,
                InputShareMessage::Frame {
                    session_id,
                    token,
                    target_display: "display".into(),
                    sequence: 2,
                    topology_generation: receiver_topology.generation,
                    events: vec![SharedInputEvent::Button {
                        button: 0,
                        pressed: true,
                    }],
                },
            );
            assert!(controller.leases.is_empty());
            assert!(receiver.leases.is_empty());
            assert!(
                !std::iter::from_fn(|| events.try_recv().ok())
                    .chain(std::iter::from_fn(|| controller_events.try_recv().ok()))
                    .any(|event| matches!(
                        event,
                        RuntimeEvent::SharedInputReceived { .. }
                            | RuntimeEvent::InputControlGranted { .. }
                    ))
            );
        }
    }
    #[tokio::test]
    async fn disabled_controller_only_establishes_metadata_links_for_selected_peers() {
        let (mut actor, peer, _, transport, _) = setup(true);
        actor.snapshot.settings.enabled = false;
        actor.available = false;
        actor.snapshot.settings.participants.clear();
        transport.contexts.lock().unwrap().clear();
        actor.tick();
        tokio::task::yield_now().await;
        assert!(transport.ensured.lock().unwrap().is_empty());

        actor.snapshot.settings.participants.push(peer);
        actor.tick();
        tokio::task::yield_now().await;
        assert_eq!(*transport.ensured.lock().unwrap(), vec![peer]);
        actor.tick();
        tokio::task::yield_now().await;
        assert_eq!(*transport.ensured.lock().unwrap(), vec![peer]);
        assert!(actor.leases.is_empty());
        assert!(transport.sent.lock().unwrap().is_empty());
    }
    #[test]
    fn metadata_still_requires_physical_lan_and_current_network_membership() {
        for non_lan in [true, false] {
            let (mut actor, peer, mut context, transport, _) = setup(false);
            actor.snapshot.settings = InputSharingSettings::default();
            actor.available = false;
            let topology = actor.snapshot.remote_topologies.remove(&peer).unwrap();
            if non_lan {
                context.scope = LinkScope::Overlay;
                context.interface_kind = InterfaceKind::Vpn;
                *transport.contexts.lock().unwrap() = vec![context.clone()];
            } else {
                let mut view = actor.network.borrow().clone();
                view.device_network.integrity_error = Some("bad roster signature".into());
                let (_, network) = watch::channel(view);
                actor.network = network;
            }
            receive(
                &mut actor,
                peer,
                &context,
                InputShareMessage::DisplayTopology(topology),
            );
            assert!(actor.snapshot.remote_topologies.is_empty());
            assert!(transport.sent.lock().unwrap().is_empty());
        }
    }
}
