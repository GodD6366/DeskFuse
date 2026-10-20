use super::*;

#[derive(Clone, Debug)]
pub(super) struct Activation {
    pub id: InputActivationId,
    pub intent_id: InputActivationId,
    pub peer: DeviceId,
    pub context: Option<ConnectionContext>,
    pub status: InputActivationStatus,
    pub reason: Option<String>,
    pub sequence: u64,
    pub ack_sequence: u64,
    pub suppressed: bool,
    sent: bool,
    /// 连续多少轮激活请求没有得到对方任何应答，用于退避重发。
    unanswered: u32,
    deadline: Instant,
    continuous_deadline_ms: u64,
    renew_at: Instant,
}

impl Activation {
    #[cfg(test)]
    pub(super) fn ready_fixture(peer: DeviceId, context: ConnectionContext) -> Self {
        let mut activation = Self::new(peer);
        activation.context = Some(context);
        activation.status = InputActivationStatus::Ready;
        activation.sent = true;
        activation.sequence = 1;
        activation.ack_sequence = 1;
        activation.renew_at = Instant::now() + RENEW;
        activation
    }
    pub(super) fn new(peer: DeviceId) -> Self {
        Self {
            id: InputActivationId::new(),
            intent_id: InputActivationId::new(),
            peer,
            context: None,
            status: InputActivationStatus::Preparing,
            reason: None,
            sequence: 0,
            ack_sequence: 0,
            suppressed: false,
            sent: false,
            unanswered: 0,
            deadline: Instant::now() + TTL,
            continuous_deadline_ms: lease_now_ms().checked_add(1500).unwrap_or(0),
            renew_at: Instant::now(),
        }
    }
    pub(super) fn expired(&self) -> bool {
        self.sent
            && (Instant::now() >= self.deadline || lease_now_ms() >= self.continuous_deadline_ms)
    }
    pub(super) fn ready(&self) -> bool {
        self.sent
            && !self.suppressed
            && !self.expired()
            && self.status == InputActivationStatus::Ready
    }
    fn extend(&mut self) {
        self.deadline = Instant::now() + TTL;
        self.continuous_deadline_ms = lease_now_ms().checked_add(1500).unwrap_or(0);
    }
}

/// 对方一直不应答（例如它认为本机不在线而丢弃请求）时的重发间隔：
/// 1、2、4、8 秒封顶，避免每 1.5 秒无限重建激活刷满日志。
fn unanswered_backoff(unanswered: u32) -> Duration {
    match unanswered {
        0 => Duration::ZERO,
        n => Duration::from_secs(1u64 << (n - 1).min(3)),
    }
}

pub(super) struct RetiredActivation {
    id: InputActivationId,
    peer: DeviceId,
    status: InputActivationStatus,
}

impl InputActor {
    pub(super) fn activation_diagnostic(
        &mut self,
        peer: DeviceId,
        stage: &'static str,
        detail: impl FnOnce() -> String,
    ) {
        if !deskfuse_core::debug_logging::enabled() {
            return;
        }
        let detail = detail();
        let key = (peer, stage);
        if self
            .diagnostic_states
            .get(&key)
            .is_some_and(|(previous, at)| {
                previous == &detail && at.elapsed() < Duration::from_secs(30)
            })
        {
            return;
        }
        // Bound log bookkeeping separately from the safety histories it measures.
        if self.diagnostic_states.len() >= 256 && !self.diagnostic_states.contains_key(&key) {
            self.diagnostic_states.clear();
        }
        self.diagnostic_states
            .insert(key, (detail.clone(), Instant::now()));
        // Diagnostics must never consume the native input delivery queue.
        let _ = self.events.send(RuntimeEvent::InputDiagnostic {
            stage,
            detail: format!(
                "peer={peer} retired_count={} paused_count={} history_capacity=1024 role={:?} enabled={} {detail}",
                self.retired_activations.len(), self.paused_intents.len(),
                self.snapshot.settings.role, self.snapshot.settings.enabled,
            ),
        });
    }

    pub(super) fn policy_for_settings(
        &self,
        settings: &InputSharingSettings,
    ) -> InputActivationPolicy {
        let mut policy = InputActivationPolicy {
            environment_rules: self.environment.rules.clone(),
            controller_intents: self.controller_intents.clone(),
            paused_intents: self.paused_intents.clone(),
        };
        let desired = if settings.feature_enabled
            && settings.enabled
            && settings.role == InputShareRole::Controller
        {
            &settings.participants[..]
        } else {
            &[]
        };
        policy
            .controller_intents
            .retain(|peer, _| desired.contains(peer));
        for peer in desired {
            policy.controller_intents.entry(*peer).or_default();
        }
        policy
    }
    pub(super) async fn load_activation_policy(&mut self) {
        let store = self.store.clone();
        let loaded = tokio::task::spawn_blocking(move || store.get_input_activation_policy()).await;
        match loaded {
            Ok(Ok(policy)) => {
                let existing = policy.clone();
                let policy = policy.unwrap_or_default();
                if let Err(error) = policy.validate() {
                    self.policy_failed(error.to_string());
                    return;
                }
                self.environment.rules = policy.environment_rules;
                self.controller_intents = policy.controller_intents;
                self.paused_intents = policy.paused_intents;
                let reconciled = self.policy_for_settings(&self.snapshot.settings);
                if existing.as_ref() != Some(&reconciled) {
                    self.controller_intents = reconciled.controller_intents;
                    self.persist_activation_policy().await;
                }
            }
            Ok(Err(error)) => self.policy_failed(error.to_string()),
            Err(error) => self.policy_failed(error.to_string()),
        }
    }
    pub(super) fn policy_failed(&mut self, reason: String) {
        let local = self.network.borrow().snapshot.local_device_id;
        self.activation_diagnostic(local, "activation_policy_failed", || {
            format!("reason={reason}")
        });
        self.activation_policy_error = Some(reason.clone());
        self.deactivate_all("activation policy persistence failed");
        self.emit(RuntimeEvent::Error(RuntimeError::Store(reason)));
    }
    pub(super) async fn persist_activation_policy(&mut self) {
        // Serialize explicit policy/settings writes behind the pending periodic
        // save, so an older snapshot can never overwrite a newer user decision.
        self.finish_policy_save().await;
        let policy = InputActivationPolicy {
            environment_rules: self.environment.rules.clone(),
            controller_intents: self.controller_intents.clone(),
            paused_intents: self.paused_intents.clone(),
        };
        let store = self.store.clone();
        match tokio::task::spawn_blocking(move || store.set_input_activation_policy(policy)).await {
            Ok(Ok(())) => {
                if self.activation_policy_error.take().is_some() {
                    // A transient store failure must not disable automatic
                    // activation until the whole app restarts.
                    let local = self.network.borrow().snapshot.local_device_id;
                    self.activation_diagnostic(local, "activation_policy_recovered", || {
                        "policy store is writable again".to_string()
                    });
                }
            }
            Ok(Err(error)) => self.policy_failed(error.to_string()),
            Err(error) => self.policy_failed(error.to_string()),
        }
    }
    fn activation_capable(&self, peer: DeviceId, context: &ConnectionContext) -> bool {
        self.network_allowed(peer)
            && self.context_live(peer, context)
            && self
                .transport
                .input_capabilities(peer, context.session_id)
                .contains(Capabilities::INPUT_ACTIVATION)
    }
    fn retire_activation(&mut self, activation: &Activation) {
        if self
            .retired_activations
            .iter()
            .any(|a| a.id == activation.id)
        {
            return;
        }
        let Some(_) = activation.context else {
            return;
        };
        // IDs remain retired even after their QUIC context closes. Never evict
        // an ID that could otherwise be replayed on a replacement connection.
        if self.retired_activations.len() >= 1024 {
            return;
        }
        self.retired_activations.push_back(RetiredActivation {
            id: activation.id,
            peer: activation.peer,
            status: if activation.status == InputActivationStatus::PausedByUser {
                InputActivationStatus::PausedByUser
            } else {
                InputActivationStatus::Unavailable
            },
        });
        self.activation_diagnostic(activation.peer, "activation_retired", || {
            format!(
                "activation={} intent={} status={:?}",
                activation.id, activation.intent_id, activation.status
            )
        });
    }
    fn release_activation_input(&mut self, id: InputActivationId, reason: &str) {
        let leases = self
            .leases
            .iter()
            .filter(|(_, lease)| lease.activation_id == id)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for lease in leases {
            self.release(lease, reason, true);
        }
    }
    pub(super) fn lease_activation_live(&self, lease: &Lease) -> bool {
        let activation = if lease.controller {
            self.outgoing.get(&lease.peer)
        } else {
            self.incoming.as_ref()
        };
        activation.is_some_and(|a| {
            a.id == lease.activation_id
                && a.peer == lease.peer
                && a.context.as_ref() == Some(&lease.context)
                && a.ready()
        })
    }
    pub(super) fn end_incoming(&mut self, reason: &str, notify: bool) {
        if let Some(activation) = self.incoming.take() {
            self.activation_diagnostic(activation.peer, "activation_end_incoming", || {
                format!(
                    "activation={} intent={} transport={:?} reason={reason} notify={notify}",
                    activation.id,
                    activation.intent_id,
                    activation.context.as_ref().map(|c| c.session_id)
                )
            });
            self.available = false;
            self.release_activation_input(activation.id, reason);
            if notify && let Some(context) = &activation.context {
                self.send(
                    activation.peer,
                    context,
                    InputShareMessage::Deactivate {
                        activation_id: activation.id,
                        sequence: activation.sequence.max(1),
                    },
                );
            }
            self.retire_activation(&activation);
            self.snapshot.managed_control = None;
        }
    }
    fn stop_outgoing(&mut self, peer: DeviceId, reason: &str, notify: bool, remove: bool) {
        if let Some(mut activation) = self.outgoing.remove(&peer) {
            self.activation_diagnostic(peer, "activation_end_outgoing", || {
                format!("activation={} intent={} transport={:?} reason={reason} notify={notify} remove={remove}", activation.id, activation.intent_id, activation.context.as_ref().map(|c|c.session_id))
            });
            self.release_activation_input(activation.id, reason);
            if notify
                && activation.sent
                && let Some(context) = &activation.context
            {
                self.send(
                    peer,
                    context,
                    InputShareMessage::Deactivate {
                        activation_id: activation.id,
                        sequence: activation.sequence.saturating_add(1),
                    },
                );
            }
            if !remove {
                if activation.status == InputActivationStatus::PausedByUser {
                    activation.suppressed = true;
                    self.outgoing.insert(peer, activation);
                } else {
                    let mut retry = Activation::new(peer);
                    retry.intent_id = activation.intent_id;
                    retry.status = InputActivationStatus::Unavailable;
                    retry.reason = Some(reason.into());
                    // A new context is already the slow part; do not add a fixed
                    // delay on top of it. The next tick sends immediately, and
                    // the renew interval still bounds every resend after that.
                    // 只有对方从未应答过时才退避：那说明对方在丢弃请求，立即重发没有意义。
                    retry.unanswered = if activation.sent && activation.ack_sequence == 0 {
                        activation.unanswered.saturating_add(1)
                    } else {
                        0
                    };
                    retry.renew_at = Instant::now() + unanswered_backoff(retry.unanswered);
                    self.outgoing.insert(peer, retry);
                }
            }
        }
    }
    pub(super) fn deactivate_all(&mut self, reason: &str) {
        self.end_incoming(reason, true);
        for peer in self.outgoing.keys().copied().collect::<Vec<_>>() {
            self.stop_outgoing(peer, reason, true, false);
        }
        self.release_all(reason);
    }
    pub(super) fn activation_invalidated(
        &mut self,
        peer: DeviceId,
        context: TransportSessionId,
        reason: &str,
    ) {
        if self.incoming.as_ref().is_some_and(|a| {
            a.peer == peer && a.context.as_ref().is_some_and(|c| c.session_id == context)
        }) {
            self.end_incoming(reason, false);
        }
        if self
            .outgoing
            .get(&peer)
            .is_some_and(|a| a.context.as_ref().is_some_and(|c| c.session_id == context))
        {
            self.stop_outgoing(peer, reason, false, false);
        }
    }
    fn activation_reply(
        &mut self,
        peer: DeviceId,
        context: &ConnectionContext,
        id: InputActivationId,
        sequence: u64,
        status: InputActivationStatus,
        reason: Option<String>,
    ) -> bool {
        let diagnostic_reason = reason.clone();
        let sent = self.send(
            peer,
            context,
            InputShareMessage::Activated {
                activation_id: id,
                ttl_ms: 1500,
                sequence,
                status,
                reason,
            },
        );
        self.activation_diagnostic(peer, "activation_response", || {
            format!("activation={id} transport={} status={status:?} reason={diagnostic_reason:?} sent={sent}", context.session_id)
        });
        sent
    }
    pub(super) fn receive_activate(
        &mut self,
        peer: DeviceId,
        context: ConnectionContext,
        id: InputActivationId,
        intent_id: InputActivationId,
        sequence: u64,
        force: bool,
    ) {
        if !self.activation_capable(peer, &context) {
            self.activation_diagnostic(peer, "activation_rejected", || {
                format!("activation={id} intent={intent_id} transport={} reason=activation_capability_unavailable", context.session_id)
            });
            return;
        }
        if let Some(reason) = self.activation_policy_error.clone() {
            self.activation_reply(
                peer,
                &context,
                id,
                sequence,
                InputActivationStatus::Unavailable,
                Some(reason.chars().take(200).collect()),
            );
            return;
        }
        if self.paused_intents.contains(&(peer, intent_id)) {
            self.activation_reply(
                peer,
                &context,
                id,
                sequence,
                InputActivationStatus::PausedByUser,
                Some("Input was reclaimed locally for this enable cycle".into()),
            );
            return;
        }
        if self.incoming.as_ref().is_some_and(Activation::expired) {
            self.end_incoming("managed activation expired", false);
        }
        if self.incoming.as_ref().is_some_and(|current| {
            current.peer == peer && current.suppressed && current.intent_id != intent_id
        }) {
            self.end_incoming("controller began a new explicit enable cycle", false);
        }
        if let Some(retired) = self.retired_activations.iter().find(|a| a.id == id) {
            if retired.peer == peer {
                if retired.status == InputActivationStatus::PausedByUser {
                    self.activation_reply(
                        peer,
                        &context,
                        id,
                        sequence,
                        retired.status,
                        Some("Input was reclaimed locally for this enable cycle".into()),
                    );
                } else {
                    // A terminal activation must not keep acknowledging heartbeats:
                    // that renews the controller's dead ID forever. End it explicitly
                    // so the existing reconnect path creates a fresh activation.
                    self.send(
                        peer,
                        &context,
                        InputShareMessage::Deactivate {
                            activation_id: id,
                            sequence,
                        },
                    );
                }
            }
            return;
        }
        // A configured controller with no selected devices is idle: adding it
        // from another controller should make it a receiver automatically.
        // Do not displace an active controller with selected participants or
        // outgoing activations, which could otherwise create mutual control.
        // However, if force is true, allow the incoming activation to take over.
        if !force
            && self.snapshot.settings.enabled
            && self.snapshot.settings.role == InputShareRole::Controller
            && !self.snapshot.environment.local_suspended
            && (!self.snapshot.settings.participants.is_empty() || !self.outgoing.is_empty())
        {
            self.activation_reply(
                peer,
                &context,
                id,
                sequence,
                InputActivationStatus::BusyLocalController,
                Some("This device is already a controller".into()),
            );
            return;
        }
        // 强制模式：停止所有出站激活，变成被控设备
        if force
            && self.snapshot.settings.enabled
            && self.snapshot.settings.role == InputShareRole::Controller
            && (!self.snapshot.settings.participants.is_empty() || !self.outgoing.is_empty())
        {
            for peer_id in self.outgoing.keys().copied().collect::<Vec<_>>() {
                self.stop_outgoing(peer_id, "被远程设备强制接管控制权", true, true);
            }
        }
        if let Some(current) = &self.incoming {
            if current.id == id
                && (current.peer != peer || current.context.as_ref() != Some(&context))
            {
                return;
            }
            if current.id != id {
                self.activation_reply(
                    peer,
                    &context,
                    id,
                    sequence,
                    InputActivationStatus::BusyOtherController,
                    Some("Another controller already manages this device".into()),
                );
                return;
            }
            if sequence <= current.sequence {
                return;
            }
            if current.intent_id != intent_id {
                return;
            }
            if current.suppressed {
                return;
            }
        } else {
            if self.retired_activations.len() >= 1024
                || (self.paused_intents.len() >= 1024
                    && !self
                        .paused_intents
                        .iter()
                        .any(|(paused_peer, _)| *paused_peer == peer))
            {
                self.activation_diagnostic(peer, "activation_history_exhausted", || {
                    format!(
                        "activation={id} intent={intent_id} transport={}",
                        context.session_id
                    )
                });
                self.activation_reply(
                    peer,
                    &context,
                    id,
                    sequence,
                    InputActivationStatus::Unavailable,
                    Some("Local activation safety history capacity reached".into()),
                );
                return;
            }
            let mut incoming = Activation::new(peer);
            incoming.id = id;
            incoming.intent_id = intent_id;
            incoming.context = Some(context.clone());
            incoming.sent = true;
            self.incoming = Some(incoming);
            self.activation_diagnostic(peer, "activation_receive", || {
                format!("activation={id} intent={intent_id} transport={} local={} remote={} force={force}", context.session_id, context.local_addr, context.remote_addr)
            });
            self.recent_managed_intent = Some((peer, intent_id));
            // A previous local role's availability is not proof that adapters
            // have been prepared for this activation ID.
            self.available = false;
        }
        let incoming = self.incoming.as_mut().unwrap();
        incoming.sequence = sequence;
        incoming.extend();
        let (status, reason) = (incoming.status, incoming.reason.clone());
        if !self.activation_reply(peer, &context, id, sequence, status, reason) {
            self.end_incoming("activation acknowledgement failed", false);
        }
    }
    pub(super) fn receive_activated(
        &mut self,
        peer: DeviceId,
        context: ConnectionContext,
        id: InputActivationId,
        sequence: u64,
        status: InputActivationStatus,
        reason: Option<String>,
    ) {
        if !self.activation_capable(peer, &context) {
            self.activation_diagnostic(peer, "activation_ack_ignored", || {
                format!(
                    "activation={id} transport={} reason=activation_capability_unavailable",
                    context.session_id
                )
            });
            return;
        }
        let Some(activation) = self.outgoing.get_mut(&peer) else {
            self.activation_diagnostic(peer, "activation_ack_ignored", || {
                format!(
                    "activation={id} transport={} reason=no_outgoing_activation",
                    context.session_id
                )
            });
            return;
        };
        if activation.id != id
            || activation.context.as_ref() != Some(&context)
            || activation.suppressed
            || activation.expired()
            || !activation.sent
            || sequence > activation.sequence
            || sequence < activation.ack_sequence
        {
            let diagnostic = format!(
                "activation={id} expected_activation={} transport={} id_matches={} context_matches={} suppressed={} expired={} sent={} sequence_ahead={} sequence_stale={}",
                activation.id,
                context.session_id,
                activation.id == id,
                activation.context.as_ref() == Some(&context),
                activation.suppressed,
                activation.expired(),
                activation.sent,
                sequence > activation.sequence,
                sequence < activation.ack_sequence,
            );
            self.activation_diagnostic(peer, "activation_ack_ignored", || diagnostic);
            return;
        }
        // An asynchronous emergency pause is terminal even if the latest
        // heartbeat was already acknowledged; duplicate Ready never renews.
        if sequence == activation.ack_sequence && status != InputActivationStatus::PausedByUser {
            return;
        }
        activation.status = status;
        activation.reason = reason;
        activation.ack_sequence = sequence;
        if status == InputActivationStatus::PausedByUser {
            activation.suppressed = true;
        } else {
            activation.extend();
        }
        let diagnostic_reason = activation.reason.clone();
        self.activation_diagnostic(peer, "activation_ack", || {
            format!(
                "activation={id} transport={} status={status:?} reason={diagnostic_reason:?}",
                context.session_id
            )
        });
        if status != InputActivationStatus::Ready {
            self.release_activation_input(id, "managed input is not ready");
        }
    }
    pub(super) fn receive_deactivate(
        &mut self,
        peer: DeviceId,
        context: ConnectionContext,
        id: InputActivationId,
        _sequence: u64,
    ) {
        if !self.activation_capable(peer, &context) {
            return;
        }
        if self
            .incoming
            .as_ref()
            .is_some_and(|a| a.id == id && a.peer == peer && a.context.as_ref() == Some(&context))
        {
            self.end_incoming("controller stopped managing input", false);
        } else if self
            .outgoing
            .get(&peer)
            .is_some_and(|a| a.id == id && a.context.as_ref() == Some(&context))
        {
            self.stop_outgoing(
                peer,
                "receiver ended managed input; reconnecting",
                false,
                false,
            );
        }
    }
    pub(super) fn set_managed_availability(
        &mut self,
        id: InputActivationId,
        status: InputActivationStatus,
        reason: Option<String>,
    ) {
        if self.activation_policy_error.is_some()
            || !status.is_native_status()
            || reason.as_ref().is_some_and(|r| r.len() > 256)
        {
            return;
        }
        let Some(incoming) = self.incoming.as_mut() else {
            return;
        };
        if incoming.id != id || incoming.expired() || incoming.suppressed {
            return;
        }
        incoming.status = status;
        incoming.reason = reason.clone();
        let peer = incoming.peer;
        self.activation_diagnostic(peer, "activation_native_status", || {
            format!("activation={id} status={status:?} reason={reason:?}")
        });
        self.available = status == InputActivationStatus::Ready;
        self.unavailable_reason = reason;
        if !self.available {
            self.release_activation_input(id, "managed native input unavailable");
        }
    }
    fn remember_paused_intent(&mut self, peer: DeviceId, intent: InputActivationId) {
        // A correct controller durably retains its current enable-cycle intent.
        // Only a new local pause supersedes this peer's previous paused cycle.
        self.paused_intents
            .retain(|(existing, _)| *existing != peer);
        self.paused_intents.insert((peer, intent));
    }
    pub(super) async fn pause_managed(&mut self, id: InputActivationId) {
        let Some(incoming) = self.incoming.as_mut() else {
            return;
        };
        if incoming.id != id {
            return;
        }
        incoming.status = InputActivationStatus::PausedByUser;
        incoming.reason =
            Some("Input was reclaimed locally; stop and restart sharing to resume".into());
        incoming.suppressed = true;
        let paused = incoming.clone();
        self.remember_paused_intent(paused.peer, paused.intent_id);
        self.available = false;
        self.release_activation_input(id, "managed input paused locally");
        self.retire_activation(&paused);
        self.persist_activation_policy().await;
        if self.activation_policy_error.is_none()
            && let Some(context) = &paused.context
        {
            self.activation_reply(
                paused.peer,
                context,
                id,
                paused.sequence.max(1),
                paused.status,
                paused.reason.clone(),
            );
        }
    }
    pub(super) async fn pause_current_managed(&mut self) {
        if let Some(current) = &self.incoming {
            self.pause_managed(current.id).await;
        } else if let Some(intent) = self.recent_managed_intent {
            self.remember_paused_intent(intent.0, intent.1);
            self.available = false;
            self.release_all("managed input paused during reconnect");
            self.persist_activation_policy().await;
        }
    }
    /// Start a fresh explicit enable cycle for one peer. A receiver that
    /// reclaimed input locally retires the current intent, so the automatic
    /// path keeps answering PausedByUser and the controller suppresses the
    /// activation forever. Only a new intent can resume that device, which is
    /// why this exists instead of asking the user to toggle the whole switch.
    pub(super) async fn restart_activation(&mut self, peer: DeviceId) {
        let settings = &self.snapshot.settings;
        if !settings.enabled
            || settings.role != InputShareRole::Controller
            || !settings.participants.contains(&peer)
        {
            return;
        }
        if let Some(activation) = self.outgoing.remove(&peer) {
            self.release_activation_input(activation.id, "user restarted managed input");
        }
        self.controller_intents
            .insert(peer, InputActivationId::new());
        let intent = self.controller_intents[&peer];
        self.activation_diagnostic(peer, "activation_restart", || format!("intent={intent}"));
        // Persist before sending: a crash between the two must not restore an
        // intent the receiver already retired.
        self.persist_activation_policy().await;
        if self.activation_policy_error.is_some() {
            return;
        }
        self.tick_activations();
        self.publish();
    }
    pub(super) fn tick_activations(&mut self) {
        if self.activation_policy_error.is_some() {
            return;
        }
        if self.incoming.as_ref().is_some_and(|a| {
            a.expired()
                || !self.network_allowed(a.peer)
                || a.context
                    .as_ref()
                    .is_none_or(|c| !self.activation_capable(a.peer, c))
        }) {
            self.end_incoming("managed activation expired or connection changed", true);
        }
        let settings = &self.snapshot.settings;
        let desired = if settings.enabled
            && settings.role == InputShareRole::Controller
            && !self.snapshot.environment.local_suspended
            && self.incoming.is_none()
        {
            settings.participants.to_vec()
        } else {
            Vec::new()
        };
        for peer in self.outgoing.keys().copied().collect::<Vec<_>>() {
            if !desired.contains(&peer) {
                self.stop_outgoing(peer, "controller deselected or disabled input", true, true);
            }
        }
        for peer in desired {
            let Some(intent_id) = self.controller_intents.get(&peer).copied() else {
                continue;
            };
            self.outgoing.entry(peer).or_insert_with(|| {
                let mut activation = Activation::new(peer);
                activation.intent_id = intent_id;
                activation
            });
            let activation = &self.outgoing[&peer];
            if activation.suppressed {
                continue;
            }
            if activation.sent
                && (activation.expired()
                    || !self.network_allowed(peer)
                    || activation
                        .context
                        .as_ref()
                        .is_none_or(|c| !self.activation_capable(peer, c)))
            {
                let reason = if activation.ack_sequence == 0 {
                    "peer did not acknowledge activation; retrying"
                } else {
                    "activation expired or LAN changed; reconnecting"
                };
                self.stop_outgoing(peer, reason, true, false);
                continue;
            }
            if !self.network_allowed(peer) {
                continue;
            }
            let context = activation.context.clone().or_else(|| {
                self.transport
                    .input_contexts(peer)
                    .into_iter()
                    .find(|context| self.activation_capable(peer, context))
            });
            let Some(context) = context else {
                if !self.transport.input_contexts(peer).is_empty() {
                    let activation = self.outgoing.get_mut(&peer).unwrap();
                    activation.status = InputActivationStatus::Unavailable;
                    activation.reason =
                        Some("Update both devices to support automatic input activation".into());
                    let id = activation.id;
                    self.activation_diagnostic(peer, "activation_capability_missing", || {
                        format!("activation={id} reason=input_activation_not_negotiated")
                    });
                }
                continue;
            };
            let activation = self.outgoing.get_mut(&peer).unwrap();
            if Instant::now() < activation.renew_at {
                continue;
            }
            if !activation.sent {
                activation.extend();
                activation.status = InputActivationStatus::Preparing;
                activation.reason = None;
            }
            activation.context = Some(context.clone());
            activation.sequence = activation.sequence.saturating_add(1);
            activation.sent = true;
            activation.renew_at = Instant::now() + RENEW;
            let message = InputShareMessage::Activate {
                activation_id: activation.id,
                intent_id: activation.intent_id,
                ttl_ms: 1500,
                sequence: activation.sequence,
                force: false,
            };
            let id = activation.id;
            let intent = activation.intent_id;
            let status = activation.status;
            self.activation_diagnostic(peer, "activation_send", || {
                format!("activation={id} intent={intent} transport={} local={} remote={} status={status:?}", context.session_id, context.local_addr, context.remote_addr)
            });
            if !self.send(peer, &context, message) {
                self.stop_outgoing(peer, "activation send failed", false, false);
            }
        }
    }
    pub(super) fn publish_activations(&mut self) {
        self.snapshot.managed_control = self.incoming.as_ref().and_then(|a| {
            a.context.clone().map(|context| ManagedInputControl {
                activation_id: a.id,
                controller: a.peer,
                context,
                status: a.status,
                reason: a.reason.clone(),
            })
        });
        self.snapshot.activations = self
            .outgoing
            .iter()
            .map(|(peer, a)| {
                (
                    *peer,
                    InputActivationState {
                        activation_id: a.id,
                        status: a.status,
                        reason: a.reason.clone(),
                    },
                )
            })
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{receive, receiver_grant, setup};
    use super::*;
    use std::result::Result;
    struct PolicyFaultStore {
        inner: crate::MemoryRuntimeStore,
        transport: Arc<super::super::tests::TestTransport>,
        fail_read: bool,
        fail_write: bool,
    }
    impl RuntimeStore for PolicyFaultStore {
        fn get_input_activation_policy(
            &self,
        ) -> Result<Option<InputActivationPolicy>, RuntimeError> {
            if self.fail_read {
                Err(RuntimeError::Store("bad policy row".into()))
            } else {
                self.inner.get_input_activation_policy()
            }
        }
        fn set_input_activation_policy(
            &self,
            policy: InputActivationPolicy,
        ) -> Result<(), RuntimeError> {
            if !policy.paused_intents.is_empty() {
                assert!(
                    !self
                        .transport
                        .sent
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|(_, _, message)| matches!(
                            message,
                            InputShareMessage::Activated {
                                status: InputActivationStatus::PausedByUser,
                                ..
                            }
                        )),
                    "pause ACK preceded durable commit"
                );
            }
            if self.fail_write {
                Err(RuntimeError::Store("policy disk full".into()))
            } else {
                self.inner.set_input_activation_policy(policy)
            }
        }
        fn list_peers(&self) -> Result<Vec<crate::PeerRecord>, RuntimeError> {
            self.inner.list_peers()
        }
        fn upsert_peer(&self, peer: crate::PeerRecord) -> Result<(), RuntimeError> {
            self.inner.upsert_peer(peer)
        }
        fn delete_peer(&self, peer: DeviceId) -> Result<bool, RuntimeError> {
            self.inner.delete_peer(peer)
        }
        fn list_manual_endpoints(
            &self,
            peer: DeviceId,
        ) -> Result<Vec<crate::ManualEndpoint>, RuntimeError> {
            self.inner.list_manual_endpoints(peer)
        }

        fn get_settings(&self) -> Result<crate::Settings, RuntimeError> {
            self.inner.get_settings()
        }
    }
    #[tokio::test]
    async fn pause_ack_follows_durable_commit_and_policy_failures_block_activation() {
        for fail_write in [false, true] {
            let (mut actor, peer, context, transport, _) = setup(false);
            let current = actor.incoming.as_ref().unwrap().clone();
            actor.store = Arc::new(PolicyFaultStore {
                inner: crate::MemoryRuntimeStore::default(),
                transport: transport.clone(),
                fail_read: false,
                fail_write,
            });
            actor
                .command(RuntimeCommand::PauseCurrentManagedActivation)
                .await;
            assert!(!actor.available);
            let pause_ack = transport
                .sent
                .lock()
                .unwrap()
                .iter()
                .any(|(_, _, message)| {
                    matches!(
                        message,
                        InputShareMessage::Activated {
                            status: InputActivationStatus::PausedByUser,
                            ..
                        }
                    )
                });
            assert_eq!(pause_ack, !fail_write);
            if fail_write {
                assert!(actor.activation_policy_error.is_some());
                offer(
                    &mut actor,
                    peer,
                    &context,
                    InputActivationId::new(),
                    InputActivationId::new(),
                    1,
                );
                assert!(actor.incoming.is_none());
            } else {
                assert!(
                    actor
                        .store
                        .get_input_activation_policy()
                        .unwrap()
                        .unwrap()
                        .paused_intents
                        .contains(&(peer, current.intent_id))
                );
            }
        }
        let (mut actor, peer, context, transport, _) = setup(false);
        actor.store = Arc::new(PolicyFaultStore {
            inner: crate::MemoryRuntimeStore::default(),
            transport,
            fail_read: true,
            fail_write: false,
        });
        actor.load_activation_policy().await;
        offer(
            &mut actor,
            peer,
            &context,
            InputActivationId::new(),
            InputActivationId::new(),
            1,
        );
        assert!(actor.activation_policy_error.is_some());
        assert!(actor.incoming.is_none());
        assert!(actor.leases.is_empty());
    }

    #[test]
    fn unanswered_activation_backs_off_instead_of_spinning() {
        let (mut actor, peer, _, _, _) = setup(true);
        for expected in [1, 2, 4, 8, 8] {
            let activation = actor.outgoing.get_mut(&peer).unwrap();
            activation.sent = true;
            activation.ack_sequence = 0;
            activation.deadline = Instant::now() - Duration::from_millis(1);
            actor.tick_activations();
            let retry = &actor.outgoing[&peer];
            let wait = retry.renew_at.saturating_duration_since(Instant::now());
            assert!(
                wait > Duration::from_secs(expected) - Duration::from_millis(200)
                    && wait <= Duration::from_secs(expected),
                "unanswered={} wait={wait:?}",
                retry.unanswered
            );
            assert_eq!(
                retry.reason.as_deref(),
                Some("peer did not acknowledge activation; retrying")
            );
        }
        // 对方应答过的激活过期（换网等）仍立即重建，不受退避影响。
        let activation = actor.outgoing.get_mut(&peer).unwrap();
        activation.sent = true;
        activation.ack_sequence = 3;
        activation.deadline = Instant::now() - Duration::from_millis(1);
        actor.tick_activations();
        let retry = &actor.outgoing[&peer];
        assert_eq!(retry.unanswered, 0);
        assert!(retry.renew_at <= Instant::now());
    }

    fn offer(
        actor: &mut InputActor,
        peer: DeviceId,
        context: &ConnectionContext,
        id: InputActivationId,
        intent: InputActivationId,
        sequence: u64,
    ) {
        receive(
            actor,
            peer,
            context,
            InputShareMessage::Activate {
                activation_id: id,
                intent_id: intent,
                ttl_ms: 1500,
                sequence,
                force: false,
            },
        );
    }
    fn add_peer(actor: &mut InputActor) -> DeviceId {
        let mut view = actor.network.borrow().clone();
        let mut peer = view.snapshot.peers[0].clone();
        peer.id = DeviceId::from_public_key([10; 32]).unwrap();
        peer.public_key = [10; 32];
        let id = peer.id;
        view.snapshot.peers.push(peer);
        let (_, network) = watch::channel(view);
        actor.network = network;
        id
    }
    #[test]
    fn idle_controller_becomes_receiver_when_added_by_another_controller() {
        let (mut actor, peer, context, _, _) = setup(true);
        actor.snapshot.settings.participants.clear();
        actor.outgoing.clear();
        let id = InputActivationId::new();
        offer(&mut actor, peer, &context, id, InputActivationId::new(), 1);
        assert_eq!(actor.incoming.as_ref().unwrap().id, id);
        actor.tick_activations();
        assert!(actor.outgoing.is_empty());
        actor.publish();
        assert_eq!(
            actor.snapshot.effective_settings().role,
            InputShareRole::Controlled
        );
    }

    #[test]
    fn explicit_participant_activates_through_wifi_vote_conflict() {
        let (mut controller, peer, _, transport, _) = setup(true);
        controller.outgoing.clear();
        controller.snapshot.environment.conflict = true;
        controller.tick_activations();
        let (id, intent) = match &transport.sent.lock().unwrap().last().unwrap().2 {
            InputShareMessage::Activate {
                activation_id,
                intent_id,
                ..
            } => (*activation_id, *intent_id),
            other => panic!("expected an activation for the selected device: {other:?}"),
        };

        let (mut receiver, receiver_peer, context, _, _) = setup(false);
        receiver.incoming = None;
        let other = add_peer(&mut receiver);
        receiver.snapshot.environment.conflict = true;
        receiver.snapshot.environment.selected = Some(other);
        offer(&mut receiver, receiver_peer, &context, id, intent, 1);
        assert_eq!(receiver.incoming.as_ref().unwrap().peer, receiver_peer);
        // Subsequent Wi-Fi votes cannot discard this live activation.
        receiver.tick_environment();
        assert_eq!(receiver.incoming.as_ref().unwrap().peer, receiver_peer);
        assert_eq!(controller.outgoing[&peer].id, id);
    }

    #[tokio::test]
    async fn master_gate_releases_receiver_and_rejects_remote_reactivation_until_reenabled() {
        let (mut actor, peer, context, _, _) = setup(false);
        let mut settings = actor.snapshot.settings.clone();
        settings.feature_enabled = false;
        actor
            .command(RuntimeCommand::SetInputSharingSettings(settings.clone()))
            .await;
        assert!(actor.incoming.is_none());
        assert!(actor.leases.is_empty());
        offer(
            &mut actor,
            peer,
            &context,
            InputActivationId::new(),
            InputActivationId::new(),
            1,
        );
        actor.tick();
        assert!(actor.incoming.is_none());
        assert!(!actor.snapshot.effective_settings().enabled);
        settings.feature_enabled = true;
        actor
            .command(RuntimeCommand::SetInputSharingSettings(settings))
            .await;
        offer(
            &mut actor,
            peer,
            &context,
            InputActivationId::new(),
            InputActivationId::new(),
            1,
        );
        assert!(actor.incoming.is_some());
    }

    #[tokio::test]
    async fn master_gate_preserves_controller_configuration_and_resumes() {
        let (mut actor, peer, _, _, _) = setup(true);
        let original = actor.snapshot.settings.clone();
        let mut settings = original.clone();
        settings.feature_enabled = false;
        actor
            .command(RuntimeCommand::SetInputSharingSettings(settings))
            .await;
        assert!(actor.outgoing.is_empty());
        assert!(actor.leases.is_empty());
        assert_eq!(actor.snapshot.settings.participants, original.participants);
        assert_eq!(actor.snapshot.settings.role, original.role);
        assert!(actor.snapshot.settings.enabled);
        actor.tick();
        assert!(actor.outgoing.is_empty());
        actor
            .command(RuntimeCommand::SetInputSharingSettings(original))
            .await;
        assert!(actor.outgoing.contains_key(&peer));
    }

    #[tokio::test]
    async fn disabled_receiver_prepares_and_requires_matching_native_ready_without_persisting() {
        let (mut actor, peer, context, transport, _) = setup(false);
        actor.incoming = None;
        actor.available = false;
        actor.snapshot.scenes.push(
            InputSharingScene::new("local scene".into(), actor.snapshot.settings.clone()).unwrap(),
        );
        let original = (
            actor.snapshot.settings.clone(),
            actor.snapshot.scenes.clone(),
        );
        let id = InputActivationId::new();
        let intent = InputActivationId::new();
        offer(&mut actor, peer, &context, id, intent, 1);
        actor.publish();
        assert_eq!(
            actor.snapshot.managed_control.as_ref().unwrap().status,
            InputActivationStatus::Preparing
        );
        assert_eq!(
            actor.snapshot.effective_settings().role,
            InputShareRole::Controlled
        );
        assert!(actor.snapshot.effective_settings().enabled);
        assert!(!actor.available);
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Request {
                session_id: InputSessionId::new(),
                target_display: "display".into(),
            },
        );
        assert!(actor.leases.is_empty());
        actor
            .command(RuntimeCommand::SetInputAvailability {
                available: true,
                reason: None,
            })
            .await;
        actor
            .command(RuntimeCommand::SetManagedInputAvailability {
                activation_id: InputActivationId::new(),
                status: InputActivationStatus::Ready,
                reason: None,
            })
            .await;
        assert!(!actor.available);
        actor
            .command(RuntimeCommand::SetManagedInputAvailability {
                activation_id: id,
                status: InputActivationStatus::WaitingAccessibility,
                reason: Some("permission missing".into()),
            })
            .await;
        offer(&mut actor, peer, &context, id, intent, 2);
        assert!(matches!(
            transport.sent.lock().unwrap().last().unwrap().2,
            InputShareMessage::Activated {
                status: InputActivationStatus::WaitingAccessibility,
                ..
            }
        ));
        assert!(actor.leases.is_empty());
        actor
            .command(RuntimeCommand::SetManagedInputAvailability {
                activation_id: id,
                status: InputActivationStatus::Ready,
                reason: None,
            })
            .await;
        let (session, _) = receiver_grant(&mut actor, peer, &context);
        assert_eq!(actor.leases[&session].activation_id, id);
        actor
            .command(RuntimeCommand::SetInputAvailability {
                available: false,
                reason: Some("temporarily unavailable".into()),
            })
            .await;
        assert!(actor.leases.is_empty());
        assert_eq!(actor.incoming.as_ref().unwrap().id, id);
        assert_eq!(
            actor.incoming.as_ref().unwrap().status,
            InputActivationStatus::Unavailable
        );
        actor
            .command(RuntimeCommand::SetInputAvailability {
                available: true,
                reason: None,
            })
            .await;
        assert!(!actor.available);
        assert_eq!(
            (
                actor.snapshot.settings.clone(),
                actor.snapshot.scenes.clone()
            ),
            original
        );
    }
    #[tokio::test]
    async fn first_managed_controller_wins_and_local_enabled_controller_takes_priority() {
        let (mut actor, peer, context, transport, _) = setup(false);
        actor.incoming = None;
        let other = add_peer(&mut actor);
        let id = InputActivationId::new();
        offer(&mut actor, peer, &context, id, InputActivationId::new(), 1);
        offer(
            &mut actor,
            other,
            &context,
            InputActivationId::new(),
            InputActivationId::new(),
            1,
        );
        assert_eq!(actor.incoming.as_ref().unwrap().peer, peer);
        assert!(matches!(
            transport.sent.lock().unwrap().last().unwrap().2,
            InputShareMessage::Activated {
                status: InputActivationStatus::BusyOtherController,
                ..
            }
        ));
        let settings = InputSharingSettings {
            enabled: true,
            role: InputShareRole::Controller,
            participants: vec![other],
            ..Default::default()
        };
        actor
            .command(RuntimeCommand::SetInputSharingSettings(settings))
            .await;
        assert!(actor.incoming.is_none());
        offer(
            &mut actor,
            other,
            &context,
            InputActivationId::new(),
            InputActivationId::new(),
            1,
        );
        assert!(matches!(
            transport.sent.lock().unwrap().last().unwrap().2,
            InputShareMessage::Activated {
                status: InputActivationStatus::BusyLocalController,
                ..
            }
        ));
    }
    #[test]
    fn activation_acks_require_exact_context_sequence_and_live_deadline() {
        let (mut actor, peer, context, transport, _) = setup(true);
        actor.outgoing.clear();
        actor.available = false;
        actor.tick_activations();
        let id = actor.outgoing[&peer].id;
        assert!(matches!(
            transport.sent.lock().unwrap().last().unwrap().2,
            InputShareMessage::Activate { .. }
        ));
        let mut other = context.clone();
        other.session_id = TransportSessionId::new();
        transport.contexts.lock().unwrap().push(other.clone());
        let ack = InputShareMessage::Activated {
            activation_id: id,
            ttl_ms: 1500,
            sequence: 1,
            status: InputActivationStatus::Ready,
            reason: None,
        };
        receive(&mut actor, peer, &other, ack.clone());
        assert!(!actor.outgoing[&peer].ready());
        receive(&mut actor, peer, &context, ack.clone());
        assert!(actor.outgoing[&peer].ready());
        actor.request(peer, "display".into());
        assert!(
            actor.leases.is_empty(),
            "controller native readiness is still required"
        );
        let deadline = actor.outgoing[&peer].deadline;
        receive(&mut actor, peer, &context, ack.clone());
        assert_eq!(actor.outgoing[&peer].deadline, deadline);
        actor
            .outgoing
            .get_mut(&peer)
            .unwrap()
            .continuous_deadline_ms = 0;
        receive(&mut actor, peer, &context, ack);
        assert!(actor.outgoing[&peer].expired());
        let intent = actor.outgoing[&peer].intent_id;
        actor.tick_activations();
        assert_ne!(actor.outgoing[&peer].id, id);
        assert_eq!(actor.outgoing[&peer].intent_id, intent);
    }
    #[test]
    fn receiver_expired_or_rebound_activation_never_revives() {
        let (mut actor, peer, context, transport, _) = setup(false);
        actor.incoming = None;
        let id = InputActivationId::new();
        let intent = InputActivationId::new();
        offer(&mut actor, peer, &context, id, intent, 1);
        let mut other = context.clone();
        other.session_id = TransportSessionId::new();
        transport.contexts.lock().unwrap().push(other.clone());
        offer(&mut actor, peer, &other, id, intent, 2);
        assert_eq!(
            actor.incoming.as_ref().unwrap().context.as_ref(),
            Some(&context)
        );
        assert_eq!(actor.incoming.as_ref().unwrap().sequence, 1);
        actor.incoming.as_mut().unwrap().continuous_deadline_ms = 0;
        offer(&mut actor, peer, &context, id, intent, 2);
        assert!(actor.incoming.is_none());
        *transport.contexts.lock().unwrap() = vec![other.clone()];
        offer(&mut actor, peer, &other, id, intent, 3);
        assert!(actor.incoming.is_none());
    }

    #[test]
    fn expired_receiver_tells_controller_to_retry_with_a_fresh_activation() {
        let (mut controller, peer, context, _, _) = setup(true);
        let (mut receiver, receiver_peer, receiver_context, transport, _) = setup(false);
        receiver.incoming = None;
        let old = controller.outgoing[&peer].clone();
        offer(
            &mut receiver,
            receiver_peer,
            &receiver_context,
            old.id,
            old.intent_id,
            1,
        );
        receiver.incoming.as_mut().unwrap().continuous_deadline_ms = 0;
        // A heartbeat can arrive after expiry but before the receiver's tick.
        // The terminal reply must not acknowledge/extend the dead activation.
        offer(
            &mut receiver,
            receiver_peer,
            &receiver_context,
            old.id,
            old.intent_id,
            2,
        );
        let reply = transport.sent.lock().unwrap().last().unwrap().2.clone();
        assert!(
            matches!(reply, InputShareMessage::Deactivate { activation_id, .. } if activation_id == old.id)
        );
        receive(&mut controller, peer, &context, reply);
        let retry = &controller.outgoing[&peer];
        assert_ne!(retry.id, old.id);
        assert_eq!(retry.intent_id, old.intent_id);
        assert!(!retry.ready());
        offer(
            &mut receiver,
            receiver_peer,
            &receiver_context,
            retry.id,
            retry.intent_id,
            1,
        );
        assert_eq!(receiver.incoming.as_ref().unwrap().id, retry.id);
        assert_eq!(
            receiver.incoming.as_ref().unwrap().status,
            InputActivationStatus::Preparing
        );
        assert!(receiver.leases.is_empty());
    }
    #[tokio::test]
    async fn controller_and_receiver_restart_preserve_pause_until_a_durable_explicit_enable_edge() {
        let (mut controller, peer, context, _, _) = setup(true);
        controller.outgoing.clear();
        controller.load_activation_policy().await;
        controller.tick_activations();
        let first = controller.outgoing[&peer].clone();
        let controller_store = controller.store.clone();
        let (mut receiver, receiver_peer, receiver_context, _, _) = setup(false);
        receiver.incoming = None;
        receiver.load_activation_policy().await;
        // Repeated explicit enable cycles and local pauses never consume one
        // durable slot per cycle. Exercise the real command and store path.
        for _ in 0..1025 {
            receiver.recent_managed_intent = Some((receiver_peer, InputActivationId::new()));
            receiver
                .command(RuntimeCommand::PauseCurrentManagedActivation)
                .await;
            assert_eq!(receiver.paused_intents.len(), 1);
            assert_eq!(
                receiver
                    .store
                    .get_input_activation_policy()
                    .unwrap()
                    .unwrap()
                    .paused_intents
                    .len(),
                1
            );
        }
        let other_peer = DeviceId::from_bytes([91; 32]);
        let other_intent = InputActivationId::new();
        receiver.recent_managed_intent = Some((other_peer, other_intent));
        receiver
            .command(RuntimeCommand::PauseCurrentManagedActivation)
            .await;
        assert_eq!(receiver.paused_intents.len(), 2);
        offer(
            &mut receiver,
            receiver_peer,
            &receiver_context,
            first.id,
            first.intent_id,
            1,
        );
        receiver
            .command(RuntimeCommand::PauseCurrentManagedActivation)
            .await;
        let receiver_store = receiver.store.clone();
        assert_eq!(receiver.paused_intents.len(), 2);
        assert!(
            receiver
                .paused_intents
                .contains(&(other_peer, other_intent))
        );
        assert!(
            receiver
                .paused_intents
                .contains(&(receiver_peer, first.intent_id))
        );

        let (mut restarted, _, _, _, _) = setup(true);
        restarted.outgoing.clear();
        restarted.store = controller_store;
        restarted.load_activation_policy().await;
        restarted.tick_activations();
        assert_eq!(restarted.outgoing[&peer].intent_id, first.intent_id);
        assert_ne!(restarted.outgoing[&peer].id, first.id);
        let (mut restarted_receiver, _, _, receiver_transport, _) = setup(false);
        let receiver_settings = restarted_receiver.snapshot.settings.clone();
        restarted_receiver.incoming = None;
        restarted_receiver.store = receiver_store;
        restarted_receiver.load_activation_policy().await;
        let retry = &restarted.outgoing[&peer];
        offer(
            &mut restarted_receiver,
            receiver_peer,
            &receiver_context,
            retry.id,
            retry.intent_id,
            1,
        );
        // Use the restarted receiver's actual live fixture context.
        let actual_context = receiver_transport.contexts.lock().unwrap()[0].clone();
        offer(
            &mut restarted_receiver,
            receiver_peer,
            &actual_context,
            retry.id,
            retry.intent_id,
            1,
        );
        assert!(restarted_receiver.incoming.is_none());
        assert!(matches!(
            receiver_transport.sent.lock().unwrap().last().unwrap().2,
            InputShareMessage::Activated {
                status: InputActivationStatus::PausedByUser,
                ..
            }
        ));
        let mut settings = restarted.snapshot.settings.clone();
        settings.enabled = false;
        restarted
            .command(RuntimeCommand::SetInputSharingSettings(settings.clone()))
            .await;
        settings.enabled = true;
        restarted
            .command(RuntimeCommand::SetInputSharingSettings(settings))
            .await;
        let fresh = &restarted.outgoing[&peer];
        assert_ne!(fresh.intent_id, first.intent_id);
        offer(
            &mut restarted_receiver,
            receiver_peer,
            &actual_context,
            fresh.id,
            fresh.intent_id,
            1,
        );
        assert_eq!(restarted_receiver.incoming.as_ref().unwrap().id, fresh.id);
        assert_eq!(restarted_receiver.snapshot.settings, receiver_settings);
        assert_eq!(
            restarted
                .store
                .get_input_activation_policy()
                .unwrap()
                .unwrap()
                .controller_intents[&peer],
            fresh.intent_id
        );
        assert!(controller.context_live(peer, &context));
    }
    #[tokio::test]
    async fn emergency_during_the_disconnected_gap_pauses_the_recent_intent() {
        let (mut receiver, peer, context, transport, _) = setup(false);
        receiver.incoming = None;
        let id = InputActivationId::new();
        let intent = InputActivationId::new();
        offer(&mut receiver, peer, &context, id, intent, 1);
        receiver.end_incoming("physical context lost", false);
        assert!(receiver.incoming.is_none());
        receiver
            .command(RuntimeCommand::PauseCurrentManagedActivation)
            .await;
        let mut next_context = context.clone();
        next_context.session_id = TransportSessionId::new();
        *transport.contexts.lock().unwrap() = vec![next_context.clone()];
        offer(
            &mut receiver,
            peer,
            &next_context,
            InputActivationId::new(),
            intent,
            1,
        );
        assert!(receiver.incoming.is_none());
        assert!(
            receiver
                .store
                .get_input_activation_policy()
                .unwrap()
                .unwrap()
                .paused_intents
                .contains(&(peer, intent))
        );
        assert!(matches!(
            transport.sent.lock().unwrap().last().unwrap().2,
            InputShareMessage::Activated {
                status: InputActivationStatus::PausedByUser,
                ..
            }
        ));
    }
    #[tokio::test]
    async fn lost_pause_ack_and_new_context_cannot_restart_the_same_enable_intent() {
        let (mut controller, peer, context, _, _) = setup(true);
        let (mut receiver, receiver_peer, receiver_context, receiver_transport, _) = setup(false);
        receiver.incoming = None;
        let old = controller.outgoing[&peer].clone();
        offer(
            &mut receiver,
            receiver_peer,
            &receiver_context,
            old.id,
            old.intent_id,
            1,
        );
        // A was replaced before the local emergency command executes: the
        // fieldless command must atomically pause current B, not a stale A.
        receiver.end_incoming("simulate transport replacement", false);
        let current = InputActivationId::new();
        offer(
            &mut receiver,
            receiver_peer,
            &receiver_context,
            current,
            old.intent_id,
            1,
        );
        receiver
            .command(RuntimeCommand::PauseCurrentManagedActivation)
            .await;
        assert_eq!(receiver.incoming.as_ref().unwrap().id, current);
        assert_eq!(
            receiver.incoming.as_ref().unwrap().status,
            InputActivationStatus::PausedByUser
        );
        receiver_transport.sent.lock().unwrap().clear(); // Drop the pause ACK.
        controller
            .outgoing
            .get_mut(&peer)
            .unwrap()
            .continuous_deadline_ms = 0;
        controller.tick_activations();
        let retry = controller.outgoing.get_mut(&peer).unwrap();
        retry.renew_at = Instant::now();
        let retry_id = retry.id;
        assert_ne!(retry_id, old.id);
        assert_eq!(retry.intent_id, old.intent_id);
        controller.tick_activations();
        let mut replacement = receiver_context.clone();
        replacement.session_id = TransportSessionId::new();
        receiver_transport
            .contexts
            .lock()
            .unwrap()
            .push(replacement.clone());
        offer(
            &mut receiver,
            receiver_peer,
            &replacement,
            retry_id,
            old.intent_id,
            1,
        );
        let (_, _, paused) = receiver_transport
            .sent
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .clone();
        assert!(matches!(
            paused,
            InputShareMessage::Activated {
                status: InputActivationStatus::PausedByUser,
                ..
            }
        ));
        receive(&mut controller, peer, &context, paused);
        assert!(controller.outgoing[&peer].suppressed);
        let mut settings = controller.snapshot.settings.clone();
        settings.switch_mode = EdgeSwitchMode::Dwell250Ms;
        controller
            .command(RuntimeCommand::SetInputSharingSettings(settings.clone()))
            .await;
        assert_eq!(controller.outgoing[&peer].id, retry_id);
        assert!(controller.outgoing[&peer].suppressed);
        settings.enabled = false;
        controller
            .command(RuntimeCommand::SetInputSharingSettings(settings.clone()))
            .await;
        settings.enabled = true;
        controller
            .command(RuntimeCommand::SetInputSharingSettings(settings))
            .await;
        let restarted = &controller.outgoing[&peer];
        assert_ne!(restarted.intent_id, old.intent_id);
        offer(
            &mut receiver,
            receiver_peer,
            &replacement,
            restarted.id,
            restarted.intent_id,
            1,
        );
        assert_eq!(receiver.incoming.as_ref().unwrap().id, restarted.id);
        assert_eq!(
            receiver.incoming.as_ref().unwrap().status,
            InputActivationStatus::Preparing
        );
    }
    #[test]
    fn only_selected_peer_is_activated_and_only_negotiated_context_supports_activation() {
        let (mut actor, peer, context, transport, _) = setup(true);
        actor.outgoing.clear();
        let other = add_peer(&mut actor);
        transport
            .capabilities
            .lock()
            .unwrap()
            .insert(context.session_id, Capabilities::INPUT_SHARING);
        actor.tick_activations();
        assert!(transport.sent.lock().unwrap().is_empty());
        assert_eq!(
            actor.outgoing[&peer].status,
            InputActivationStatus::Unavailable
        );
        let mut capable = context.clone();
        capable.session_id = TransportSessionId::new();
        transport.contexts.lock().unwrap().push(capable.clone());
        actor.tick_activations();
        let messages = transport.sent.lock().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].0, peer);
        assert_eq!(messages[0].1, capable.session_id);
        assert!(!actor.outgoing.contains_key(&other));
    }
    #[tokio::test]
    async fn off_remove_and_connection_loss_release_activation_input_but_leave_and_topology_do_not()
    {
        let (mut actor, peer, context, _, _) = setup(false);
        let id = actor.incoming.as_ref().unwrap().id;
        let settings = actor.snapshot.settings.clone();
        let (session, token) = receiver_grant(&mut actor, peer, &context);
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Leave {
                session_id: session,
                token,
                sequence: 1,
            },
        );
        assert_eq!(actor.incoming.as_ref().unwrap().id, id);
        let mut topology = actor.snapshot.local_topology.clone().unwrap();
        topology.generation += 1;
        actor
            .command(RuntimeCommand::PublishDisplayTopology(topology))
            .await;
        assert!(actor.leases.is_empty());
        assert_eq!(actor.incoming.as_ref().unwrap().id, id);
        let _ = receiver_grant(&mut actor, peer, &context);
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Deactivate {
                activation_id: id,
                sequence: 2,
            },
        );
        assert!(actor.incoming.is_none());
        assert!(actor.leases.is_empty());
        assert_eq!(actor.snapshot.settings, settings);
        for remove in [false, true] {
            let (mut sender, peer, _, _, _) = setup(true);
            let intent = sender.outgoing[&peer].intent_id;
            let mut settings = sender.snapshot.settings.clone();
            if remove {
                settings.participants.clear();
            } else {
                settings.enabled = false;
            }
            // Exercise the persisted settings command, not a direct mutation
            // that bypasses controller-intent reconciliation.
            sender
                .command(RuntimeCommand::SetInputSharingSettings(settings.clone()))
                .await;
            sender.tick_activations();
            assert!(sender.outgoing.is_empty());
            settings.enabled = true;
            settings.participants = vec![peer];
            sender
                .command(RuntimeCommand::SetInputSharingSettings(settings))
                .await;
            sender.tick_activations();
            assert_ne!(sender.outgoing[&peer].intent_id, intent);
        }
    }
    #[test]
    fn force_activate_overrides_local_controller() {
        let (mut controller, _peer, context, transport, _) = setup(true);
        // 验证A是主控且有选中的参与者
        assert!(controller.snapshot.settings.enabled);
        assert_eq!(
            controller.snapshot.settings.role,
            InputShareRole::Controller
        );
        assert!(!controller.snapshot.settings.participants.is_empty());
        assert!(!controller.outgoing.is_empty());

        // 添加另一个peer到网络中
        let other_peer = add_peer(&mut controller);

        // B设备尝试非强制控制，应该被拒绝
        let id = InputActivationId::new();
        let intent = InputActivationId::new();
        receive(
            &mut controller,
            other_peer,
            &context,
            InputShareMessage::Activate {
                activation_id: id,
                intent_id: intent,
                ttl_ms: 1500,
                sequence: 1,
                force: false,
            },
        );
        assert!(controller.incoming.is_none());
        assert!(matches!(
            transport.sent.lock().unwrap().last().unwrap().2,
            InputShareMessage::Activated {
                status: InputActivationStatus::BusyLocalController,
                ..
            }
        ));

        // B设备发送强制控制请求
        let force_id = InputActivationId::new();
        let force_intent = InputActivationId::new();
        receive(
            &mut controller,
            other_peer,
            &context,
            InputShareMessage::Activate {
                activation_id: force_id,
                intent_id: force_intent,
                ttl_ms: 1500,
                sequence: 1,
                force: true,
            },
        );

        // 验证A已变成被控设备
        assert!(controller.incoming.is_some());
        assert_eq!(controller.incoming.as_ref().unwrap().id, force_id);
        assert_eq!(controller.incoming.as_ref().unwrap().peer, other_peer);
        // 所有出站激活应该被停止
        assert!(
            controller.outgoing.is_empty() || controller.outgoing.values().all(|a| a.suppressed)
        );
        // 验证发送了成功的激活确认
        assert!(matches!(
            transport.sent.lock().unwrap().last().unwrap().2,
            InputShareMessage::Activated {
                activation_id,
                status: InputActivationStatus::Preparing,
                ..
            } if activation_id == force_id
        ));
    }
}
