//! Environment policy never overwrites saved controller settings or screen layouts.
use super::*;
use std::collections::BTreeSet;

#[cfg(test)]
mod tests;

#[derive(Clone)]
struct Presence {
    controller: bool,
    ssid: Option<String>,
    preferred: Option<DeviceId>,
    seen: Instant,
}
#[derive(Default)]
pub(super) struct Environment {
    pub(super) rules: BTreeMap<String, DeviceId>,
    pub(super) temporary: Option<DeviceId>,
    pub(super) dirty: bool,
    observed: Option<String>,
    pending: Option<(Option<String>, Instant)>,
    peers: BTreeMap<DeviceId, Presence>,
    last_sent: Option<Instant>,
}
impl InputActor {
    fn environment_capable(&self, peer: DeviceId, context: &ConnectionContext) -> bool {
        self.network_allowed(peer)
            && self.context_live(peer, context)
            && self
                .transport
                .input_capabilities(peer, context.session_id)
                .contains(Capabilities::INPUT_ENVIRONMENT)
    }
    pub(super) fn observe_environment(&mut self, ssid: Option<String>) {
        self.environment.observed = ssid.filter(|s| !s.is_empty() && s.len() <= 32);
    }
    pub(super) fn receive_controller_presence(
        &mut self,
        peer: DeviceId,
        context: &ConnectionContext,
        controller: bool,
        ssid: Option<String>,
        preferred: Option<DeviceId>,
    ) {
        if !self.environment_capable(peer, context) {
            return;
        }
        self.environment.peers.insert(
            peer,
            Presence {
                controller,
                ssid,
                preferred,
                seen: Instant::now(),
            },
        );
    }
    pub(super) fn receive_controller_selection(
        &mut self,
        peer: DeviceId,
        context: &ConnectionContext,
        controller: DeviceId,
        ssid: Option<String>,
        remember: bool,
    ) {
        if !self.environment_capable(peer, context) {
            return;
        }
        // A device the user enabled as a controller here stays the controller.
        // A remote device cannot suspend that role; only an explicit local
        // handover (choose_environment_controller) may do that.
        if self.snapshot.settings.enabled
            && self.snapshot.settings.role == InputShareRole::Controller
        {
            return;
        }
        // A wired Mac may adopt its trusted Wi-Fi peer's scene. A different
        // locally observed SSID must not be overwritten by a remembered remote
        // rule; an explicit one-time handover can resolve differing SSIDs.
        if remember
            && self
                .environment
                .observed
                .as_ref()
                .is_some_and(|local| Some(local) != ssid.as_ref())
        {
            return;
        }
        if remember && ssid != self.snapshot.environment.ssid && self.environment.observed.is_some()
        {
            return;
        }
        let local = self.network.borrow().snapshot.local_device_id;
        if controller != local && !self.network_allowed(controller) {
            return;
        }
        self.environment.temporary = Some(controller);
        if remember
            && let Some(ssid) = ssid
            && (self.environment.rules.contains_key(&ssid) || self.environment.rules.len() < 64)
        {
            self.environment.rules.insert(ssid, controller);
            self.environment.dirty = true;
        }
        self.environment.last_sent = None;
    }
    pub(super) fn start_environment_save(&mut self) {
        if !self.environment.dirty
            || self.pending_policy_save.is_some()
            || self.activation_policy_error.is_some()
        {
            return;
        }
        self.environment.dirty = false;
        let policy = InputActivationPolicy {
            environment_rules: self.environment.rules.clone(),
            controller_intents: self.controller_intents.clone(),
            paused_intents: self.paused_intents.clone(),
        };
        let store = self.store.clone();
        self.pending_policy_save = Some(tokio::task::spawn_blocking(move || {
            store.set_input_activation_policy(policy)
        }));
    }

    pub(super) async fn finish_policy_save(&mut self) {
        let Some(task) = self.pending_policy_save.take() else {
            return;
        };
        match task.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => self.policy_failed(error.to_string()),
            Err(error) => self.policy_failed(error.to_string()),
        }
    }

    pub(super) async fn persist_environment_if_dirty(&mut self) {
        if self.environment.dirty {
            self.environment.dirty = false;
            self.persist_activation_policy().await;
        }
    }
    pub(super) async fn choose_environment_controller(
        &mut self,
        controller: DeviceId,
        remember: bool,
    ) {
        let local = self.network.borrow().snapshot.local_device_id;
        if (!self.snapshot.environment.candidates.contains(&controller)
            && !(controller == local && self.snapshot.settings.role == InputShareRole::Controller))
            || (controller != local && !self.network_allowed(controller))
        {
            self.settings_error("所选主控已离线或尚未启用主控，请重新选择");
            return;
        }
        if remember && self.environment.pending.is_some() {
            self.settings_error("Wi-Fi 场景正在变化，请等待网络稳定后保存规则");
            return;
        }
        let ssid = self.snapshot.environment.ssid.clone();
        if remember && ssid.is_none() {
            self.settings_error("尚未取得唯一的 Wi-Fi SSID，请使用本次选择");
            return;
        }
        if remember && let Some(ssid) = &ssid {
            if !self.environment.rules.contains_key(ssid) && self.environment.rules.len() >= 64 {
                self.settings_error("已保存 64 个网络，请先删除不用的规则");
                return;
            }
            self.environment.rules.insert(ssid.clone(), controller);
            self.environment.dirty = true;
            self.persist_environment_if_dirty().await;
            if self.activation_policy_error.is_some() {
                return;
            }
        }
        // An explicit local handover is the only Wi-Fi selection allowed to
        // preempt a live receiver activation; passive votes never do.
        if self.incoming.as_ref().is_some_and(|a| a.peer != controller) {
            self.end_incoming("controller selected locally", true);
        }
        self.release_all("controller selected locally");
        self.environment.temporary = Some(controller);
        self.environment.last_sent = None;
        let peers = self.environment.peers.keys().copied().collect::<Vec<_>>();
        for peer in peers {
            for context in self.transport.input_contexts(peer) {
                if self.environment_capable(peer, &context) {
                    self.send(
                        peer,
                        &context,
                        InputShareMessage::ControllerSelection {
                            controller,
                            ssid: ssid.clone(),
                            remember,
                        },
                    );
                    break;
                }
            }
        }
        self.tick_environment();
    }
    pub(super) fn receive_forget_environment(
        &mut self,
        peer: DeviceId,
        context: &ConnectionContext,
        ssid: String,
    ) {
        if !self.environment_capable(peer, context) {
            return;
        }
        self.environment.rules.remove(&ssid);
        if self.snapshot.environment.ssid.as_ref() == Some(&ssid) {
            self.environment.temporary = None;
        }
        self.environment.dirty = true;
        self.environment.last_sent = None;
    }
    pub(super) async fn forget_environment_rule(&mut self, ssid: String) {
        self.environment.rules.remove(&ssid);
        if self.snapshot.environment.ssid.as_ref() == Some(&ssid) {
            self.environment.temporary = None;
        }
        self.environment.dirty = true;
        self.persist_environment_if_dirty().await;
        if self.activation_policy_error.is_some() {
            return;
        }
        self.environment.last_sent = None;
        for peer in self.environment.peers.keys().copied().collect::<Vec<_>>() {
            for context in self.transport.input_contexts(peer) {
                if self.environment_capable(peer, &context) {
                    self.send(
                        peer,
                        &context,
                        InputShareMessage::ForgetControllerEnvironment { ssid: ssid.clone() },
                    );
                    break;
                }
            }
        }
    }
    pub(super) fn tick_environment(&mut self) {
        if self.activation_policy_error.is_some() {
            return;
        }
        let now = Instant::now();
        let local = self.network.borrow().snapshot.local_device_id;
        // Keep controller intent across brief link loss; actual input leases
        // still expire independently after 1.5 seconds.
        let trusted = self
            .network
            .borrow()
            .snapshot
            .peers
            .iter()
            .filter(|p| p.trust == TrustState::Trusted)
            .map(|p| p.id)
            .collect::<BTreeSet<_>>();
        self.environment.peers.retain(|peer, p| {
            trusted.contains(peer) && now.duration_since(p.seen) < Duration::from_secs(12)
        });
        let remote_ssids = self
            .environment
            .peers
            .values()
            .filter_map(|p| p.ssid.clone())
            .collect::<BTreeSet<_>>();
        let ambiguous = self
            .environment
            .observed
            .as_ref()
            .map_or(remote_ssids.len() > 1, |local| {
                remote_ssids.iter().any(|remote| remote != local)
            });
        let ssid =
            self.environment.observed.clone().or_else(|| {
                (remote_ssids.len() == 1).then(|| remote_ssids.first().unwrap().clone())
            });
        let busy = self.leases.values().any(|lease| lease.entered);
        if ssid != self.snapshot.environment.ssid {
            let pending = self
                .environment
                .pending
                .get_or_insert_with(|| (ssid.clone(), now));
            if pending.0 != ssid {
                *pending = (ssid.clone(), now);
            }
            if now.duration_since(pending.1) >= Duration::from_secs(10) && !busy {
                self.snapshot.environment.ssid = ssid.clone();
                self.environment.temporary = None;
                self.environment.pending = None;
                // A network with a remembered arrangement restores it here; the
                // sharing switch, participants and consent are never touched.
                self.apply_environment_layout(ssid.as_deref());
            }
        } else {
            self.environment.pending = None;
        }
        let local_controller = self.snapshot.settings.enabled
            && self.snapshot.settings.role == InputShareRole::Controller;
        let mut candidates = self
            .environment
            .peers
            .iter()
            .filter(|(_, p)| p.controller)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        if local_controller {
            candidates.push(local);
        }
        candidates.sort();
        candidates.dedup();
        let own = self.environment.temporary.or_else(|| {
            self.snapshot
                .environment
                .ssid
                .as_ref()
                .and_then(|ssid| self.environment.rules.get(ssid).copied())
        });
        let mut votes = BTreeSet::new();
        if let Some(preferred) = own {
            votes.insert(preferred);
        }
        for presence in self
            .environment
            .peers
            .values()
            .filter(|_| self.environment.temporary.is_none())
        {
            if (presence.ssid.is_none() || presence.ssid == self.snapshot.environment.ssid)
                && let Some(preferred) = presence.preferred
            {
                votes.insert(preferred);
            }
        }
        let automatic = !self.environment.rules.is_empty();
        let conflict = (ambiguous && self.environment.temporary.is_none())
            || votes.len() > 1
            || (votes.is_empty() && candidates.len() > 1);
        let requested = if conflict {
            None
        } else {
            votes
                .first()
                .copied()
                .or_else(|| (!automatic && candidates.len() == 1).then(|| candidates[0]))
        };
        // A locally enabled controller is handed over only by the local user.
        // A remote vote without a local saved rule surfaces as a conflict instead of a
        // silent takeover of this device's controller role.
        let remote_request = local_controller
            && self.environment.temporary.is_none()
            && own != requested
            && requested.is_some_and(|id| id != local);
        let conflict = conflict || remote_request;
        let selected = if remote_request { None } else { requested };
        // A verified, live activation is the current controller. Wi-Fi votes
        // must not revoke its lease or switch the receiver back to a stale
        // preference; a later selection takes effect after that activation ends.
        if !busy && self.environment.pending.is_none() && self.incoming.is_none() {
            // Conflicting rules keep the existing effective controller stable,
            // but an unmatched saved scene must not retain a stale controller.
            if !conflict
                || self.snapshot.environment.selected.is_none()
                || (automatic && own.is_none() && !remote_request)
            {
                let previous = self.snapshot.environment.selected;
                let suspended = selected.is_some_and(|id| id != local)
                    || (automatic && selected != Some(local));
                if previous != selected || self.snapshot.environment.local_suspended != suspended {
                    self.release_all("controller environment changed");
                    if self
                        .incoming
                        .as_ref()
                        .is_some_and(|a| Some(a.peer) != selected)
                    {
                        self.end_incoming("environment selected another controller", true);
                    }
                }
                self.snapshot.environment.selected = selected;
                self.snapshot.environment.local_suspended = suspended;
                // 静默记忆：已知 Wi-Fi 上生效的主控直接成为保存的规则，
                // 切换主控即自动更新；关闭本机主控则清除该网络的映射，
                // 让它回到「未匹配自动暂停」语义。落盘复用 dirty + 周期 flush。
                if let Some(ssid) = self.snapshot.environment.ssid.clone() {
                    if !local_controller
                        && self.environment.temporary.is_none()
                        && self.environment.rules.get(&ssid) == Some(&local)
                    {
                        self.environment.rules.remove(&ssid);
                        self.environment.dirty = true;
                        self.environment.last_sent = None;
                    } else if let Some(controller) = selected
                        && self.environment.rules.get(&ssid) != Some(&controller)
                        && (self.environment.rules.contains_key(&ssid)
                            || self.environment.rules.len() < 64)
                    {
                        self.environment.rules.insert(ssid, controller);
                        self.environment.dirty = true;
                        self.environment.last_sent = None;
                    }
                }
            }
        }
        self.snapshot.environment.candidates = candidates;
        self.snapshot.environment.rules = self.environment.rules.clone();
        self.snapshot.environment.conflict = conflict;
        self.snapshot.environment.message = if self.environment.pending.is_some() {
            if busy {
                "Wi-Fi 场景已变化，返回本机屏幕后切换主控"
            } else {
                "正在等待 Wi-Fi 场景稳定…"
            }
        } else if conflict {
            "检测到多个主控或场景规则冲突，请选择本次主控"
        } else if self
            .snapshot
            .environment
            .selected
            .is_some_and(|id| !self.snapshot.environment.candidates.contains(&id))
        {
            "该 Wi-Fi 的首选主控不在线；保留本地操作，可临时选择其他主控"
        } else if automatic && self.snapshot.environment.selected.is_none() {
            "当前 Wi-Fi 未匹配已保存的主控规则，已暂停本机主控"
        } else if own.is_some() {
            "已应用 Wi-Fi 场景主控选择，原有主控设置与布局保持保存"
        } else {
            "可为当前 Wi-Fi 保存主控；陌生网络出现多个主控时需要选择"
        }
        .into();
        if self
            .environment
            .last_sent
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(2))
        {
            return;
        }
        self.environment.last_sent = Some(now);
        let peers = self
            .network
            .borrow()
            .snapshot
            .peers
            .iter()
            .map(|p| p.id)
            .collect::<Vec<_>>();
        for peer in peers {
            for context in self.transport.input_contexts(peer) {
                if self.environment_capable(peer, &context) {
                    self.send(
                        peer,
                        &context,
                        InputShareMessage::ControllerPresence {
                            controller: local_controller,
                            ssid: self.environment.observed.clone(),
                            preferred: own,
                        },
                    );
                    break;
                }
            }
        }
    }
}
