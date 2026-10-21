use super::*;
struct Attempt {
    id: u64,
    generation: u64,
    task: tokio::task::JoinHandle<()>,
}
struct Completion {
    id: u64,
    peer: DeviceId,
    generation: u64,
    result: std::result::Result<(), RuntimeError>,
}
struct Retry {
    generation: u64,
    failures: u32,
    due: Instant,
}
pub(super) struct Links {
    next: u64,
    active: BTreeMap<DeviceId, Attempt>,
    retries: BTreeMap<DeviceId, Retry>,
    /// 最近一次建链失败的原因，链路恢复或设备不再需要链路时清除。
    pub(super) failures: BTreeMap<DeviceId, InputLinkFailure>,
    checked: BTreeMap<DeviceId, Instant>,
    retired: Vec<tokio::task::JoinHandle<()>>,
    tx: mpsc::UnboundedSender<Completion>,
    rx: mpsc::UnboundedReceiver<Completion>,
}
impl Default for Links {
    fn default() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            next: 0,
            active: BTreeMap::new(),
            retries: BTreeMap::new(),
            failures: BTreeMap::new(),
            checked: BTreeMap::new(),
            retired: Vec::new(),
            tx,
            rx,
        }
    }
}
impl Drop for Links {
    fn drop(&mut self) {
        for attempt in self.active.values() {
            attempt.task.abort();
        }
        for task in &self.retired {
            task.abort();
        }
    }
}
impl InputActor {
    pub(super) fn maintain_link(&mut self, peer: DeviceId, endpoints: Vec<Endpoint>) {
        // Only the controller upgrades a live route. The receiver follows that
        // authenticated activation, so the two sides cannot fight over routing.
        if !self.transport.independent_input()
            || !self.snapshot.settings.enabled
            || self.snapshot.settings.role != InputShareRole::Controller
            || !self.snapshot.settings.participants.contains(&peer)
            || self.incoming.is_some()
        {
            return;
        }
        // Preparation is asynchronous, but retirement is an actor decision.
        // Recheck entered here: a drag may have begun while a probe was pending.
        if !self.leases.values().any(|l| l.peer == peer && l.entered) {
            let contexts = self.transport.input_contexts(peer);
            let best = contexts
                .iter()
                .filter(|c| {
                    c.input_eligible() && self.transport.input_motion_available(peer, c.session_id)
                })
                .min_by_key(|c| deskfuse_transport::routes::input_path_rank(c.interface_kind));
            if let Some(best) = best {
                let rank = deskfuse_transport::routes::input_path_rank(best.interface_kind);
                if contexts.iter().any(|c| {
                    c.input_eligible()
                        && deskfuse_transport::routes::input_path_rank(c.interface_kind) > rank
                }) {
                    self.transport.select_input_route(peer, best.session_id);
                }
            }
        }
        if self
            .links
            .checked
            .get(&peer)
            .is_some_and(|at| at.elapsed() < Duration::from_secs(5))
        {
            return;
        }
        self.links.checked.insert(peer, Instant::now());
        self.start_link(peer, endpoints);
    }
    fn link_desired(&self, peer: DeviceId) -> bool {
        // A selected peer may exchange layout metadata without native input
        // permission or an active controller environment. A network change can
        // suspend control while display metadata still needs to reconnect.
        // Activation and leases enforce availability separately.
        self.network_allowed(peer)
            && (self.snapshot.settings.participants.contains(&peer)
                || self.incoming.as_ref().is_some_and(|a| a.peer == peer))
    }
    pub(super) fn tick_links(&mut self) {
        let obsolete_retries = self
            .links
            .retries
            .iter()
            .filter(|(peer, retry)| {
                !self.link_desired(**peer)
                    || self.transport.control_generation(**peer).unwrap_or(0) != retry.generation
            })
            .map(|(peer, _)| *peer)
            .collect::<Vec<_>>();
        for peer in obsolete_retries {
            self.links.retries.remove(&peer);
        }
        let stale_failures = self
            .links
            .failures
            .keys()
            .filter(|peer| !self.link_desired(**peer))
            .copied()
            .collect::<Vec<_>>();
        for peer in stale_failures {
            self.links.failures.remove(&peer);
        }
        let obsolete = self
            .links
            .active
            .iter()
            .filter(|(peer, a)| {
                !self.link_desired(**peer)
                    || self.transport.control_generation(**peer).unwrap_or(0) != a.generation
            })
            .map(|(p, _)| *p)
            .collect::<Vec<_>>();
        for peer in obsolete {
            if let Some(a) = self.links.active.remove(&peer) {
                a.task.abort();
                self.links.retired.push(a.task);
            }
            self.links.retries.remove(&peer);
        }
        while let Ok(done) = self.links.rx.try_recv() {
            if !self
                .links
                .active
                .get(&done.peer)
                .is_some_and(|a| a.generation == done.generation && a.id == done.id)
            {
                continue;
            }
            if let Some(a) = self.links.active.remove(&done.peer) {
                self.links.retired.push(a.task);
            }
            if !self.link_desired(done.peer)
                || self.transport.control_generation(done.peer).unwrap_or(0) != done.generation
            {
                continue;
            }
            match done.result {
                Ok(()) => {
                    let contexts = self.transport.input_contexts(done.peer);
                    self.activation_diagnostic(done.peer, "input_link_recovery_result", || {
                        format!("generation={} contexts={contexts:?}", done.generation)
                    });
                    if self
                        .transport
                        .input_contexts(done.peer)
                        .iter()
                        .any(ConnectionContext::input_eligible)
                    {
                        self.links.retries.remove(&done.peer);
                        self.links.failures.remove(&done.peer);
                    } else {
                        // A provider acknowledgement is not yet an eligible
                        // physical LAN link. Avoid an immediate retry loop.
                        self.links.retries.insert(
                            done.peer,
                            Retry {
                                generation: done.generation,
                                failures: 0,
                                due: Instant::now() + Duration::from_secs(2),
                            },
                        );
                    }
                    self.attempted.remove(&done.peer);
                }
                Err(error) => {
                    let retry = self.links.retries.entry(done.peer).or_insert(Retry {
                        generation: done.generation,
                        failures: 0,
                        due: Instant::now(),
                    });
                    let delay = (1u64 << retry.failures.min(5)).min(30);
                    retry.failures = retry.failures.saturating_add(1);
                    retry.due = Instant::now() + Duration::from_secs(delay);
                    let failure = match &error {
                        RuntimeError::InputLinkUnavailable { failure, .. } => *failure,
                        _ => InputLinkFailure::Other,
                    };
                    self.links.failures.insert(done.peer, failure);
                    self.activation_diagnostic(done.peer, "input_link_recovery_failed", || {
                        format!(
                            "generation={} retry_after_secs={delay} error={error}",
                            done.generation
                        )
                    });
                    if !self
                        .transport
                        .input_contexts(done.peer)
                        .iter()
                        .any(ConnectionContext::input_eligible)
                    {
                        self.emit(RuntimeEvent::InputLinkUnavailable {
                            peer_id: done.peer,
                            reason: format!("输入连接恢复失败，{delay} 秒后重试：{error}"),
                        });
                    }
                }
            }
        }
    }
    pub(super) fn start_link(&mut self, peer: DeviceId, endpoints: Vec<Endpoint>) {
        if !self.link_desired(peer)
            || self.links.active.contains_key(&peer)
            || self.links.retries.get(&peer).is_some_and(|retry| {
                retry.generation == self.transport.control_generation(peer).unwrap_or(0)
                    && Instant::now() < retry.due
            })
        {
            return;
        }
        let generation = self.transport.control_generation(peer).unwrap_or(0);
        let diagnostic_endpoints = endpoints.clone();
        self.activation_diagnostic(peer, "input_link_recovery_started", || {
            format!("generation={generation} endpoints={diagnostic_endpoints:?}")
        });
        self.links.next += 1;
        let id = self.links.next;
        let tx = self.links.tx.clone();
        let transport = self.transport.clone();
        let recover = async move {
            let result = tokio::time::timeout(
                Duration::from_secs(12),
                transport.ensure_input_link(peer, endpoints),
            )
            .await
            .unwrap_or_else(|_| {
                Err(RuntimeError::Transport(
                    "input link recovery timeout".into(),
                ))
            });
            let _ = tx.send(Completion {
                id,
                peer,
                generation,
                result,
            });
        };
        // Link recovery is discovery work: address resolution, a 3 s dial timeout
        // per candidate and interface inspection. Spawning it here put all of
        // that on this actor's single worker - the same worker whose turn rate is
        // the ceiling on delivered motion - so it runs on the probe executor.
        let task = match deskfuse_transport::input_executor::probe() {
            Ok(executor) => executor.spawn(recover),
            Err(_) => tokio::spawn(recover),
        };
        self.links.active.insert(
            peer,
            Attempt {
                id,
                generation,
                task,
            },
        );
    }
    pub(super) async fn reap_links(&mut self) {
        for task in std::mem::take(&mut self.links.retired) {
            if task.is_finished() {
                let _ = task.await;
            } else {
                self.links.retired.push(task);
            }
        }
    }
    pub(super) async fn stop_links(&mut self) {
        for (_, a) in std::mem::take(&mut self.links.active) {
            a.task.abort();
            self.links.retired.push(a.task);
        }
        for task in std::mem::take(&mut self.links.retired) {
            let _ = task.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_control_generation_discards_old_input_backoff() {
        let (mut actor, peer, _, _, _) = super::super::tests::setup(true);
        let current = actor.transport.control_generation(peer).unwrap_or(0);
        actor.links.retries.insert(
            peer,
            Retry {
                generation: current.wrapping_add(1),
                failures: 5,
                due: Instant::now() + Duration::from_secs(30),
            },
        );
        actor.tick_links();
        assert!(!actor.links.retries.contains_key(&peer));
        actor.links.retries.insert(
            peer,
            Retry {
                generation: current,
                failures: 5,
                due: Instant::now() + Duration::from_secs(30),
            },
        );
        actor.tick_links();
        assert!(actor.links.retries.contains_key(&peer));
        actor.snapshot.settings.participants.clear();
        actor.tick_links();
        assert!(!actor.links.retries.contains_key(&peer));
    }

    #[test]
    fn suspended_controller_keeps_selected_peer_metadata_recovery() {
        let (mut actor, peer, _, _, _) = super::super::tests::setup(true);
        actor.snapshot.environment.local_suspended = true;
        actor.snapshot.remote_topologies.clear();
        assert!(actor.link_desired(peer));
        assert!(!actor.role_allowed(peer, true));

        // Pausing the network scene must not discard a pending metadata retry.
        let generation = actor.transport.control_generation(peer).unwrap_or(0);
        actor.links.retries.insert(
            peer,
            Retry {
                generation,
                failures: 1,
                due: Instant::now() + Duration::from_secs(2),
            },
        );
        actor.tick_links();
        assert!(actor.links.retries.contains_key(&peer));

        actor.snapshot.settings.enabled = false;
        assert!(actor.link_desired(peer));
        assert!(!actor.role_allowed(peer, true));
        actor.snapshot.settings.participants.clear();
        assert!(!actor.link_desired(peer));
        actor.tick_links();
        assert!(!actor.links.retries.contains_key(&peer));
    }

    #[tokio::test]
    async fn deselected_peer_cancels_and_joins_metadata_link_reconstruction() {
        let (mut actor, peer, _, _, _) = super::super::tests::setup(true);
        let cancelled = Arc::new(AtomicBool::new(false));
        struct Cancelled(Arc<AtomicBool>);
        impl Drop for Cancelled {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let observed = cancelled.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _guard = Cancelled(observed);
            let _ = started.send(());
            std::future::pending::<()>().await;
        });
        ready.await.unwrap();
        actor.links.active.insert(
            peer,
            Attempt {
                id: 1,
                generation: 0,
                task,
            },
        );
        // Permission loss alone may retain selected-peer display metadata;
        // deselection removes that explicit peer scope and cancels the task.
        actor.snapshot.settings.participants.clear();
        actor.tick_links();
        actor.reap_links().await;
        assert!(actor.links.active.is_empty());
        // The hot loop no longer waits for aborted tasks. Shutdown is the
        // explicit join boundary and still guarantees cancellation cleanup.
        actor.stop_links().await;
        assert!(actor.links.retired.is_empty());
        assert!(cancelled.load(Ordering::Acquire));
    }
}
