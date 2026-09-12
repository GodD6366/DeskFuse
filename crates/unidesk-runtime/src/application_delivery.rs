//! Session-owned application writers and health probes. Neither blocks the actor.
use super::*;
use std::time::Duration;
use tokio::time::Instant;

// Includes QUIC delivery, not just a local buffer write. A 1 MiB file frame
// may legitimately take more than five seconds on a congested physical LAN.
pub(super) const APPLICATION_SEND_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const APPLICATION_RECEIPT_TIMEOUT: Duration = Duration::from_secs(40);

struct SendJob {
    message: PeerMessage,
    request: Option<unidesk_core::RequestId>,
}
struct Writer {
    generation: u64,
    sender: mpsc::Sender<SendJob>,
    task: task::JoinHandle<()>,
}
struct Probe {
    generation: u64,
    next: Instant,
    failures: u8,
    task: Option<task::JoinHandle<(Duration, Result<(), RuntimeError>)>>,
}
pub(super) struct Receipt {
    pub sender: tokio::sync::oneshot::Sender<Result<(), RuntimeError>>,
    pub cancel: watch::Sender<bool>,
}
pub(super) type Receipts = Arc<Mutex<BTreeMap<unidesk_core::RequestId, Receipt>>>;
pub(super) struct ReceiptGuard {
    pub receipts: Receipts,
    pub id: unidesk_core::RequestId,
}
impl Drop for ReceiptGuard {
    fn drop(&mut self) {
        if let Ok(mut results) = self.receipts.lock() {
            if let Some(receipt) = results.remove(&self.id) {
                receipt.cancel.send_replace(true);
            }
        }
    }
}
#[derive(Default)]
pub(super) struct Delivery {
    pub results: Receipts,
    writers: BTreeMap<DeviceId, Writer>,
    probes: BTreeMap<DeviceId, Probe>,
}
impl Delivery {
    pub(super) fn probes_wake(&mut self) {
        for probe in self.probes.values_mut() {
            probe.next = Instant::now();
        }
    }
    pub async fn stop(&mut self) {
        for (_, writer) in std::mem::take(&mut self.writers) {
            writer.task.abort();
            let _ = writer.task.await;
        }
        for (_, probe) in std::mem::take(&mut self.probes) {
            if let Some(task) = probe.task {
                task.abort();
                let _ = task.await;
            }
        }
    }
}
impl Drop for Delivery {
    fn drop(&mut self) {
        for writer in self.writers.values() {
            writer.task.abort();
        }
        for probe in self.probes.values() {
            if let Some(task) = &probe.task {
                task.abort();
            }
        }
    }
}
fn report(
    receipts: &Receipts,
    events: &broadcast::Sender<RuntimeEvent>,
    peer_id: DeviceId,
    generation: u64,
    job: &SendJob,
    result: Result<(), RuntimeError>,
) {
    if let Some(request_id) = job.request {
        if let Some(sender) = receipts.lock().ok().and_then(|mut r| r.remove(&request_id)) {
            let _ = sender.sender.send(result.clone());
        }
        let sequence = match &job.message {
            PeerMessage::Clipboard(p) => p.sequence,
            _ => 0,
        };
        let sync_id = match &job.message {
            PeerMessage::Clipboard(p) if p.mime == unidesk_protocol::clipboard_sync::MIME => {
                unidesk_protocol::clipboard_sync::SyncMessage::decode_checked(&p.data)
                    .ok()
                    .and_then(|m| m.sync_id.try_into().ok())
            }
            _ => None,
        };
        let _ = events.send(RuntimeEvent::ClipboardSent {
            peer_id,
            generation,
            request_id,
            sync_id,
            sequence,
            result,
        });
    } else if let Err(error) = result {
        let _ = events.send(RuntimeEvent::Error(error));
    }
}
impl Actor {
    pub(super) fn enqueue_application(
        &mut self,
        peer: DeviceId,
        generation: u64,
        message: PeerMessage,
        request: Option<unidesk_core::RequestId>,
    ) {
        if request.is_some_and(|id| {
            !self
                .delivery
                .results
                .lock()
                .is_ok_and(|r| r.contains_key(&id))
        }) {
            return;
        }
        let job = SendJob { message, request };
        let allowed = self
            .device_network
            .view(&self.blocked)
            .allows(self.identity.device_id(), peer)
            && self.peers.get(&peer).is_some_and(|p| {
                p.trust == TrustState::Trusted && p.connection == ConnectionState::Connected
            })
            && self
                .transport
                .control_generation(peer)
                .is_none_or(|current| current == generation);
        if !allowed {
            report(
                &self.delivery.results,
                &self.event_tx,
                peer,
                generation,
                &job,
                Err(RuntimeError::Transport(
                    "clipboard connection is no longer current".into(),
                )),
            );
            return;
        }
        if self
            .delivery
            .writers
            .get(&peer)
            .is_some_and(|w| w.generation != generation || w.task.is_finished())
        {
            if let Some(old) = self.delivery.writers.remove(&peer) {
                old.task.abort();
            }
        }
        let receipts = self.delivery.results.clone();
        let writer = self.delivery.writers.entry(peer).or_insert_with(|| {
            let (sender, mut receiver) = mpsc::channel::<SendJob>(8);
            let transport = self.transport.clone();
            let events = self.event_tx.clone();
            let receipts=receipts.clone();
            let task = tokio::spawn(async move {
                while let Some(job) = receiver.recv().await {
                    let cancellation=job.request.and_then(|id|receipts.lock().ok().and_then(|r|r.get(&id).map(|r|r.cancel.subscribe())));
                    if job.request.is_some() && cancellation.is_none() {continue;}
                    let cancelled=async move {
                        if let Some(mut cancel)=cancellation {
                            if !*cancel.borrow() {let _=cancel.changed().await;}
                        } else {std::future::pending::<()>().await;}
                    };
                    let result=tokio::select! {
                        biased;
                        _=cancelled=>continue,
                        result=tokio::time::timeout(APPLICATION_SEND_TIMEOUT, transport.send_message_on_session(peer, generation, job.message.clone()))=>result.unwrap_or_else(|_| Err(RuntimeError::Transport("application stream stalled".into()))),
                    };
                    let failed = result.is_err();
                    report(&receipts, &events, peer, generation, &job, result);
                    if failed {
                        let _ = transport.disconnect_control(peer, generation).await;
                        while let Ok(job) = receiver.try_recv() {
                            report(&receipts, &events, peer, generation, &job, Err(RuntimeError::Transport("application session closed".into())));
                        }
                        break;
                    }
                }
            });
            Writer { generation, sender, task }
        });
        if let Err(error) = writer.sender.try_send(job) {
            report(
                &self.delivery.results,
                &self.event_tx,
                peer,
                generation,
                &error.into_inner(),
                Err(RuntimeError::Transport("application writer is busy".into())),
            );
        }
    }

    pub(super) async fn delivery_tick(&mut self) {
        let live = self
            .peers
            .iter()
            .filter(|(id, p)| {
                self.device_network
                    .view(&self.blocked)
                    .allows(self.identity.device_id(), **id)
                    && p.connection == ConnectionState::Connected
                    && p.trust == TrustState::Trusted
            })
            .filter_map(|(peer, _)| self.transport.control_generation(*peer).map(|g| (*peer, g)))
            .collect::<BTreeMap<_, _>>();
        let obsolete = self
            .delivery
            .writers
            .iter()
            .filter(|(peer, w)| live.get(peer) != Some(&w.generation))
            .map(|(p, _)| *p)
            .collect::<Vec<_>>();
        for peer in obsolete {
            if let Some(w) = self.delivery.writers.remove(&peer) {
                w.task.abort();
                let _ = w.task.await;
            }
        }
        let obsolete = self
            .delivery
            .probes
            .iter()
            .filter(|(peer, p)| live.get(peer) != Some(&p.generation))
            .map(|(p, _)| *p)
            .collect::<Vec<_>>();
        for peer in obsolete {
            if let Some(p) = self.delivery.probes.remove(&peer) {
                if let Some(t) = p.task {
                    t.abort();
                    let _ = t.await;
                }
            }
        }
        for (peer, generation) in live {
            let probe = self.delivery.probes.entry(peer).or_insert_with(|| Probe {
                generation,
                next: Instant::now(),
                failures: 0,
                task: None,
            });
            if probe.task.as_ref().is_some_and(|t| t.is_finished()) {
                let (elapsed, result) = probe
                    .task
                    .take()
                    .expect("finished probe")
                    .await
                    .unwrap_or_else(|error| {
                        (
                            Duration::ZERO,
                            Err(RuntimeError::Transport(format!(
                                "control probe task failed: {error}"
                            ))),
                        )
                    });
                let previous_failures = probe.failures;
                let phase_and_reason = match result {
                    Ok(()) => {
                        probe.failures = 0;
                        (previous_failures > 0).then(|| {
                            (
                                "probe_recovered",
                                format!(
                                    "generation={generation} elapsed_ms={}",
                                    elapsed.as_millis()
                                ),
                            )
                        })
                    }
                    Err(error) => {
                        probe.failures = probe.failures.saturating_add(1);
                        Some((
                            "probe_failed",
                            format!(
                                "generation={generation} elapsed_ms={} error={error}",
                                elapsed.as_millis()
                            ),
                        ))
                    }
                };
                if let Some((phase, reason)) = phase_and_reason {
                    let _ = self.event_tx.send(RuntimeEvent::ConnectionRecovery {
                        peer_id: peer,
                        phase,
                        route_id: None,
                        attempt: u32::from(probe.failures),
                        retry_after_secs: None,
                        reason: Some(reason),
                    });
                }
                if probe.failures >= 2 {
                    let _ = self.transport.disconnect_control(peer, generation).await;
                    continue;
                }
            }
            if probe.task.is_none() && Instant::now() >= probe.next {
                probe.next = Instant::now() + Duration::from_secs(5);
                let transport = self.transport.clone();
                probe.task = Some(tokio::spawn(async move {
                    let started = Instant::now();
                    let result = tokio::time::timeout(
                        Duration::from_secs(2),
                        transport.probe_control(peer, generation),
                    )
                    .await
                    .unwrap_or_else(|_| {
                        Err(RuntimeError::Transport("control heartbeat timeout".into()))
                    });
                    (started.elapsed(), result)
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancelling_waiter_reaches_in_flight_writer() {
        let receipts = Receipts::default();
        let id = unidesk_core::RequestId::new();
        let (sender, _receiver) = tokio::sync::oneshot::channel();
        let (cancel, mut watcher) = watch::channel(false);
        receipts
            .lock()
            .unwrap()
            .insert(id, Receipt { sender, cancel });
        drop(ReceiptGuard {
            receipts: receipts.clone(),
            id,
        });
        watcher.changed().await.unwrap();
        assert!(*watcher.borrow());
        assert!(receipts.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn result_does_not_depend_on_lossy_ui_broadcast() {
        let receipts = Receipts::default();
        let id = unidesk_core::RequestId::new();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        receipts.lock().unwrap().insert(
            id,
            Receipt {
                sender,
                cancel: watch::channel(false).0,
            },
        );
        let (events, _) = broadcast::channel(1);
        let job = SendJob {
            message: PeerMessage::Clipboard(
                ClipboardPayload::new("text/plain", b"test".to_vec(), 1).unwrap(),
            ),
            request: Some(id),
        };
        report(
            &receipts,
            &events,
            DeviceId::from_bytes([1; 32]),
            4,
            &job,
            Ok(()),
        );
        assert!(receiver.await.unwrap().is_ok());
    }
}
