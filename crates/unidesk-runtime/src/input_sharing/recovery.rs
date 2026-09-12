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
pub(super) struct Links {
    next: u64,
    active: BTreeMap<DeviceId, Attempt>,
    retries: BTreeMap<DeviceId, (u32, Instant)>,
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
    fn link_desired(&self, peer: DeviceId) -> bool {
        // A selected peer may exchange layout metadata without native input
        // permission. Activation and leases enforce availability separately.
        self.network_allowed(peer)
            && !self.snapshot.environment.local_suspended
            && (self.snapshot.settings.participants.contains(&peer)
                || self.incoming.as_ref().is_some_and(|a| a.peer == peer))
    }
    pub(super) fn tick_links(&mut self) {
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
                    self.links.retries.remove(&done.peer);
                    self.attempted.remove(&done.peer);
                }
                Err(error) => {
                    let retry = self
                        .links
                        .retries
                        .entry(done.peer)
                        .or_insert((0, Instant::now()));
                    let delay = (1u64 << retry.0.min(5)).min(30);
                    retry.0 = retry.0.saturating_add(1);
                    retry.1 = Instant::now() + Duration::from_secs(delay);
                    self.emit(RuntimeEvent::InputLinkUnavailable {
                        peer_id: done.peer,
                        reason: format!("输入连接恢复失败，{delay} 秒后重试：{error}"),
                    });
                }
            }
        }
    }
    pub(super) fn start_link(&mut self, peer: DeviceId, endpoints: Vec<Endpoint>) {
        if !self.link_desired(peer)
            || self.links.active.contains_key(&peer)
            || self
                .links
                .retries
                .get(&peer)
                .is_some_and(|(_, due)| Instant::now() < *due)
        {
            return;
        }
        let generation = self.transport.control_generation(peer).unwrap_or(0);
        self.links.next += 1;
        let id = self.links.next;
        let tx = self.links.tx.clone();
        let transport = self.transport.clone();
        let task = tokio::spawn(async move {
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
        });
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
    #[tokio::test]
    async fn disabled_input_cancels_and_joins_link_reconstruction() {
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
        actor.available = false;
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
