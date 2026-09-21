//! Session-fixed hybrid input. Reliable ordering never shares a watermark with motion.
use super::*;
use deskfuse_input::MotionReceiver;

const SNAPSHOT_IDLE: Duration = Duration::from_millis(32);

#[derive(Clone, Debug, Default)]
pub(super) struct HybridLease {
    pub requested: bool,
    pub enabled: bool,
    pub fallback_reason: Option<String>,
    pub(super) position: Option<MotionPosition>,
    pub(super) state_epoch: u64,
    pub(super) motion_seq: u64,
    pub(super) receiver: MotionReceiver,
    pub(super) dirty_at: Option<Instant>,
}

impl InputActor {
    pub(super) fn hybrid_enter(&mut self, id: InputSessionId, target: DisplayId, x: f64, y: f64) {
        let Some(lease) = self.leases.get_mut(&id) else {
            return;
        };
        let Some(generation) = self
            .snapshot
            .remote_topologies
            .get(&lease.peer)
            .map(|t| t.generation)
        else {
            return;
        };
        let Some(sequence) = lease.sequence.checked_add(1) else {
            self.release(id, "input sequence exhausted", true);
            return;
        };
        let Some(motion_seq) = lease.hybrid.motion_seq.checked_add(1) else {
            self.release(id, "motion sequence exhausted", true);
            return;
        };
        let position = MotionPosition {
            target_display: target.clone(),
            topology_generation: generation,
            motion_seq,
            x,
            y,
        };
        lease.sequence = sequence;
        lease.entered = true;
        lease.target = target;
        lease.hybrid.position = Some(position.clone());
        lease.hybrid.motion_seq = motion_seq;
        lease.hybrid.state_epoch = sequence;
        lease.hybrid.dirty_at = None;
        let peer = lease.peer;
        let context = lease.context.clone();
        let token = lease.token.unwrap();
        self.transport
            .cancel_input_motion(peer, context.session_id, id);
        if !self.send(
            peer,
            &context,
            InputShareMessage::Hybrid {
                session_id: id,
                token,
                sequence,
                update: HybridInputUpdate::Enter { position },
            },
        ) {
            self.release(id, "hybrid entry send failed", true);
        }
    }

    pub(super) fn hybrid_events(&mut self, id: InputSessionId, events: Vec<SharedInputEvent>) {
        let mut transitions = Vec::new();
        for event in events {
            if let SharedInputEvent::Motion { x, y } = event {
                if !transitions.is_empty()
                    && !self.hybrid_transition(id, std::mem::take(&mut transitions))
                {
                    return;
                }
                let Some(lease) = self.leases.get_mut(&id) else {
                    return;
                };
                let Some(seq) = lease.hybrid.motion_seq.checked_add(1) else {
                    self.release(id, "motion sequence exhausted", true);
                    return;
                };
                let Some(position) = lease.hybrid.position.as_mut() else {
                    self.release(id, "hybrid input before entry", true);
                    return;
                };
                position.motion_seq = seq;
                position.x = x;
                position.y = y;
                lease.hybrid.motion_seq = seq;
                lease.hybrid.dirty_at = Some(Instant::now());
                let motion = MotionDatagram {
                    session_id: id,
                    token: lease.token.unwrap(),
                    state_epoch: lease.hybrid.state_epoch,
                    position: position.clone(),
                };
                if self
                    .transport
                    .send_input_motion(lease.peer, lease.context.session_id, motion)
                    .is_err()
                {
                    self.retire_motion(
                        id,
                        "motion transport unavailable; reacquire reliable control",
                    );
                    return;
                }
            } else {
                transitions.push(event);
                if transitions.len() == 254
                    && !self.hybrid_transition(id, std::mem::take(&mut transitions))
                {
                    return;
                }
            }
        }
        if !transitions.is_empty() {
            self.hybrid_transition(id, transitions);
        }
    }

    fn hybrid_transition(&mut self, id: InputSessionId, events: Vec<SharedInputEvent>) -> bool {
        let Some(lease) = self.leases.get_mut(&id) else {
            return false;
        };
        let Some(position) = lease.hybrid.position.clone() else {
            self.release(id, "hybrid transition without anchor", true);
            return false;
        };
        let Some(sequence) = lease.sequence.checked_add(1) else {
            self.release(id, "input sequence exhausted", true);
            return false;
        };
        lease.sequence = sequence;
        lease.hybrid.state_epoch = sequence;
        lease.hybrid.dirty_at = None;
        let peer = lease.peer;
        let context = lease.context.clone();
        let token = lease.token.unwrap();
        self.transport
            .cancel_input_motion(peer, context.session_id, id);
        if self.send(
            peer,
            &context,
            InputShareMessage::Hybrid {
                session_id: id,
                token,
                sequence,
                update: HybridInputUpdate::Transition { position, events },
            },
        ) {
            true
        } else {
            self.release(id, "hybrid transition send failed", true);
            false
        }
    }

    pub(super) fn flush_motion_snapshot(&mut self, id: InputSessionId) {
        let Some(lease) = self.leases.get_mut(&id) else {
            return;
        };
        if lease.expired()
            || !lease.controller
            || !lease.entered
            || !lease.hybrid.enabled
            || lease.hybrid.dirty_at.is_none()
        {
            return;
        }
        let Some(position) = lease.hybrid.position.clone() else {
            return;
        };
        let Some(sequence) = lease.sequence.checked_add(1) else {
            self.release(id, "input sequence exhausted", true);
            return;
        };
        lease.sequence = sequence;
        lease.hybrid.dirty_at = None;
        let peer = lease.peer;
        let context = lease.context.clone();
        let token = lease.token.unwrap();
        let state_epoch = lease.hybrid.state_epoch;
        if !self.send(
            peer,
            &context,
            InputShareMessage::Hybrid {
                session_id: id,
                token,
                sequence,
                update: HybridInputUpdate::Snapshot {
                    state_epoch,
                    position,
                },
            },
        ) {
            self.release(id, "final motion snapshot failed", true);
        }
    }

    pub(super) fn next_motion_snapshot_deadline(&self) -> Option<Instant> {
        self.leases
            .values()
            .filter(|l| l.controller && l.entered && l.hybrid.enabled)
            .filter_map(|l| l.hybrid.dirty_at.map(|t| t + SNAPSHOT_IDLE))
            .min()
    }

    pub(super) fn tick_motion(&mut self) {
        let now = Instant::now();
        let due = self
            .leases
            .iter()
            .filter(|(_, l)| {
                l.hybrid.enabled
                    && l.controller
                    && l.hybrid
                        .dirty_at
                        .is_some_and(|t| now.saturating_duration_since(t) >= SNAPSHOT_IDLE)
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in due {
            self.flush_motion_snapshot(id);
        }
        let failed = self
            .leases
            .iter()
            .filter(|(_, l)| {
                l.hybrid.enabled
                    && !self
                        .transport
                        .input_motion_available(l.peer, l.context.session_id)
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in failed {
            self.retire_motion(
                id,
                "motion datagram health lost; reacquire reliable control",
            );
        }
    }

    fn retire_motion(&mut self, id: InputSessionId, reason: &str) {
        if let Some(lease) = self.leases.get(&id) {
            self.motion_disabled_until
                .insert(lease.peer, Instant::now() + Duration::from_secs(30));
        }
        self.release(id, reason, true);
    }

    fn position_allowed(&self, position: &MotionPosition) -> bool {
        position.validate().is_ok()
            && self.snapshot.local_topology.as_ref().is_some_and(|t| {
                t.generation == position.topology_generation
                    && t.displays.iter().any(|d| {
                        d.id == position.target_display && d.contains(position.x, position.y)
                    })
            })
    }

    pub(super) fn inbound_motion(
        &mut self,
        peer: DeviceId,
        context: ConnectionContext,
        motion: MotionDatagram,
    ) {
        let id = motion.session_id;
        if motion.validate().is_err()
            || !self.network_allowed(peer)
            || !self.context_live(peer, &context)
            || !self.position_allowed(&motion.position)
        {
            return;
        }
        let Some(lease) = self.leases.get(&id) else {
            return;
        };
        if !lease.hybrid.enabled
            || lease.controller
            || lease.peer != peer
            || lease.context != context
            || lease.token != Some(motion.token)
            || lease.expired()
            || !self.role_allowed(peer, false)
            || !self.lease_activation_live(lease)
        {
            return;
        }
        // Future Enter may arrive after its datagram: the receiver's epoch gate
        // keeps this position pending, never authorizes injection by itself.
        let now = self.now_ms();
        let lease = self.leases.get_mut(&id).unwrap();
        let events = lease.hybrid.receiver.receive_motion(&motion, now);
        if !events.is_empty() {
            self.emit(RuntimeEvent::SharedInputReceived {
                peer_id: peer,
                session_id: id,
                target_display: motion.position.target_display,
                sequence: motion.position.motion_seq,
                state_epoch: motion.state_epoch,
                events,
            });
        }
    }

    pub(super) fn inbound_hybrid(
        &mut self,
        peer: DeviceId,
        context: ConnectionContext,
        id: InputSessionId,
        token: InputToken,
        sequence: u64,
        update: HybridInputUpdate,
    ) {
        let Some(lease) = self.leases.get(&id) else {
            return;
        };
        if !lease.hybrid.enabled
            || lease.peer != peer
            || lease.context != context
            || lease.token != Some(token)
            || lease.controller
        {
            return;
        }
        if lease.expired()
            || sequence <= lease.sequence
            || !self.role_allowed(peer, false)
            || !self.lease_activation_live(lease)
        {
            self.release(id, "hybrid control lease expired or replayed", true);
            return;
        }
        let position = match &update {
            HybridInputUpdate::Enter { position }
            | HybridInputUpdate::Transition { position, .. }
            | HybridInputUpdate::Snapshot { position, .. } => position,
        };
        if !self.position_allowed(position)
            || (!matches!(update, HybridInputUpdate::Enter { .. })
                && (!lease.entered || lease.target != position.target_display))
        {
            self.release(id, "hybrid target or topology invalid", true);
            return;
        }
        let target = position.target_display.clone();
        let now = self.now_ms();
        let lease = self.leases.get_mut(&id).unwrap();
        let events = match lease
            .hybrid
            .receiver
            .receive_reliable(sequence, &update, now)
        {
            Ok(events) => events,
            Err(_) => {
                self.release(id, "invalid hybrid state barrier", true);
                return;
            }
        };
        let motion_seq = lease.hybrid.receiver.latest_motion_seq();
        lease.sequence = sequence;
        if matches!(update, HybridInputUpdate::Enter { .. }) {
            lease.entered = true;
            lease.target = target.clone();
        }
        let state_epoch = match update {
            HybridInputUpdate::Snapshot { state_epoch, .. } => state_epoch,
            _ => sequence,
        };
        if !events.is_empty() {
            self.emit(RuntimeEvent::SharedInputReceived {
                peer_id: peer,
                session_id: id,
                target_display: target,
                sequence: motion_seq,
                state_epoch,
                events,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{TestTransport, receive, setup};
    use super::*;
    fn enable(transport: &TestTransport, context: &ConnectionContext) {
        transport.capabilities.lock().unwrap().insert(
            context.session_id,
            Capabilities::INPUT_SHARING
                | Capabilities::INPUT_ACTIVATION
                | Capabilities::INPUT_MOTION_DATAGRAM_V1,
        );
        transport.motion_ready.store(true, Ordering::Release);
    }
    fn pos(seq: u64, x: f64) -> MotionPosition {
        MotionPosition {
            target_display: "display".into(),
            topology_generation: 7,
            motion_seq: seq,
            x,
            y: 10.0,
        }
    }
    fn grant(
        actor: &mut InputActor,
        peer: DeviceId,
        context: &ConnectionContext,
    ) -> (InputSessionId, InputToken) {
        let id = InputSessionId::new();
        receive(
            actor,
            peer,
            context,
            InputShareMessage::RequestHybrid {
                session_id: id,
                target_display: "display".into(),
            },
        );
        let token = actor.leases[&id].token.unwrap();
        assert!(actor.leases[&id].hybrid.enabled);
        receive(
            actor,
            peer,
            context,
            InputShareMessage::Hybrid {
                session_id: id,
                token,
                sequence: 1,
                update: HybridInputUpdate::Enter {
                    position: pos(1, 10.0),
                },
            },
        );
        (id, token)
    }
    fn motion(
        actor: &mut InputActor,
        peer: DeviceId,
        context: &ConnectionContext,
        id: InputSessionId,
        token: InputToken,
        epoch: u64,
        position: MotionPosition,
    ) {
        actor.inbound(InputTransportEvent::Motion {
            peer_id: peer,
            context: context.clone(),
            motion: MotionDatagram {
                session_id: id,
                token,
                state_epoch: epoch,
                position,
            },
        });
    }
    fn frames(events: &InputDelivery) -> Vec<Vec<SharedInputEvent>> {
        std::iter::from_fn(|| events.try_recv().ok())
            .filter_map(|e| match e {
                RuntimeEvent::SharedInputReceived { events, .. } => Some(events),
                _ => None,
            })
            .collect()
    }
    #[test]
    fn motion_does_not_advance_reliable_sequence_or_overtake_button_barrier() {
        let (mut actor, peer, context, transport, events) = setup(false);
        enable(&transport, &context);
        let (id, token) = grant(&mut actor, peer, &context);
        frames(&events);
        motion(&mut actor, peer, &context, id, token, 2, pos(300, 30.0));
        assert!(frames(&events).is_empty());
        assert_eq!(actor.leases[&id].sequence, 1);
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Hybrid {
                session_id: id,
                token,
                sequence: 2,
                update: HybridInputUpdate::Transition {
                    position: pos(2, 20.0),
                    events: vec![SharedInputEvent::Button {
                        button: 0,
                        pressed: true,
                    }],
                },
            },
        );
        assert_eq!(
            frames(&events),
            vec![vec![
                SharedInputEvent::Motion { x: 20.0, y: 10.0 },
                SharedInputEvent::Button {
                    button: 0,
                    pressed: true
                },
                SharedInputEvent::Motion { x: 30.0, y: 10.0 }
            ]]
        );
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Renew {
                session_id: id,
                token,
                sequence: 3,
            },
        );
        assert_eq!(actor.leases[&id].sequence, 3);
        motion(&mut actor, peer, &context, id, token, 1, pos(400, 40.0));
        assert!(frames(&events).is_empty());
        assert!(actor.leases.contains_key(&id));
    }
    #[test]
    fn reliable_final_position_converges_without_rewinding_newer_datagrams() {
        let (mut actor, peer, context, transport, events) = setup(false);
        enable(&transport, &context);
        let (id, token) = grant(&mut actor, peer, &context);
        frames(&events);
        motion(&mut actor, peer, &context, id, token, 1, pos(4, 40.0));
        frames(&events);
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Hybrid {
                session_id: id,
                token,
                sequence: 2,
                update: HybridInputUpdate::Snapshot {
                    state_epoch: 1,
                    position: pos(3, 30.0),
                },
            },
        );
        assert!(frames(&events).is_empty());
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Hybrid {
                session_id: id,
                token,
                sequence: 3,
                update: HybridInputUpdate::Snapshot {
                    state_epoch: 1,
                    position: pos(5, 50.0),
                },
            },
        );
        assert_eq!(
            frames(&events),
            vec![vec![SharedInputEvent::Motion { x: 50.0, y: 10.0 }]]
        );
    }
    #[test]
    fn leave_and_release_discard_late_motion_without_reactivating_native_control() {
        let (mut actor, peer, context, transport, events) = setup(false);
        enable(&transport, &context);
        let (id, token) = grant(&mut actor, peer, &context);
        frames(&events);
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Leave {
                session_id: id,
                token,
                sequence: 2,
            },
        );
        motion(&mut actor, peer, &context, id, token, 1, pos(2, 40.0));
        assert!(frames(&events).is_empty());
        assert!(!actor.leases[&id].entered);
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Hybrid {
                session_id: id,
                token,
                sequence: 3,
                update: HybridInputUpdate::Enter {
                    position: pos(3, 20.0),
                },
            },
        );
        frames(&events);
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Release {
                session_id: id,
                token,
                sequence: 4,
            },
        );
        motion(&mut actor, peer, &context, id, token, 3, pos(4, 60.0));
        assert!(frames(&events).is_empty());
        assert!(!actor.leases.contains_key(&id));
        assert!(transport.motion_cancelled.lock().unwrap().contains(&id));
    }
    #[tokio::test]
    async fn sender_splits_mixed_frames_with_click_and_drag_anchors() {
        let (mut actor, peer, context, transport, _) = setup(true);
        enable(&transport, &context);
        actor.request(peer, "display".into());
        let id = *actor.leases.keys().next().unwrap();
        let token = InputToken::new();
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::GrantHybrid {
                session_id: id,
                token,
                ttl_ms: 1500,
                motion_enabled: true,
            },
        );
        actor.hybrid_enter(id, "display".into(), 1.0, 10.0);
        actor
            .command(RuntimeCommand::SendSharedInput {
                session_id: id,
                target_display: "display".into(),
                events: vec![
                    SharedInputEvent::Motion { x: 10.0, y: 10.0 },
                    SharedInputEvent::Button {
                        button: 0,
                        pressed: true,
                    },
                    SharedInputEvent::Motion { x: 20.0, y: 10.0 },
                    SharedInputEvent::Button {
                        button: 0,
                        pressed: false,
                    },
                    SharedInputEvent::Motion { x: 30.0, y: 10.0 },
                ],
            })
            .await;
        let motion = transport.motion_sent.lock().unwrap();
        assert_eq!(motion.len(), 3);
        assert!(
            motion[0].state_epoch < motion[1].state_epoch
                && motion[1].state_epoch < motion[2].state_epoch
        );
        drop(motion);
        let sent = transport.sent.lock().unwrap();
        let anchors = sent
            .iter()
            .filter_map(|(_, _, m)| match m {
                InputShareMessage::Hybrid {
                    update: HybridInputUpdate::Transition { position, .. },
                    ..
                } => Some(position.x),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(anchors, vec![10.0, 20.0]);
        drop(sent);
        actor.leases.get_mut(&id).unwrap().hybrid.dirty_at = Some(Instant::now() - SNAPSHOT_IDLE);
        actor.tick_motion();
        assert!(transport.sent.lock().unwrap().iter().any(|(_,_,m)|matches!(m,InputShareMessage::Hybrid{update:HybridInputUpdate::Snapshot{position,..},..} if position.x==30.0)));
    }
    #[test]
    fn fallback_mode_is_fixed_and_never_changes_on_a_renewal_grant() {
        let (mut actor, peer, context, transport, _) = setup(true);
        enable(&transport, &context);
        actor.request(peer, "display".into());
        let id = *actor.leases.keys().next().unwrap();
        let token = InputToken::new();
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::GrantHybrid {
                session_id: id,
                token,
                ttl_ms: 1500,
                motion_enabled: false,
            },
        );
        assert!(!actor.leases[&id].hybrid.enabled);
        actor.publish();
        assert_eq!(
            actor
                .snapshot
                .links
                .iter()
                .find(|p| p.peer_id == peer)
                .unwrap()
                .motion_fallback_reason
                .as_deref(),
            Some("Peer selected reliable-only")
        );
        actor.leases.get_mut(&id).unwrap().awaiting_ack = true;
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::GrantHybrid {
                session_id: id,
                token,
                ttl_ms: 1500,
                motion_enabled: true,
            },
        );
        assert!(!actor.leases[&id].hybrid.enabled);
        assert!(actor.leases[&id].awaiting_ack);
    }
    #[test]
    fn receiver_negotiates_reliable_fallback_when_datagrams_are_unavailable() {
        let (mut actor, peer, context, transport, _) = setup(false);
        enable(&transport, &context);
        transport.motion_ready.store(false, Ordering::Release);
        let id = InputSessionId::new();
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::RequestHybrid {
                session_id: id,
                target_display: "display".into(),
            },
        );
        assert!(!actor.leases[&id].hybrid.enabled);
        assert!(matches!(
            &transport.sent.lock().unwrap().last().unwrap().2,
            InputShareMessage::GrantHybrid {
                motion_enabled: false,
                ..
            }
        ));
    }
    #[test]
    fn bad_token_topology_and_legacy_frames_cannot_bypass_hybrid_gate() {
        let (mut actor, peer, context, transport, events) = setup(false);
        enable(&transport, &context);
        let (id, token) = grant(&mut actor, peer, &context);
        frames(&events);
        motion(
            &mut actor,
            peer,
            &context,
            id,
            InputToken::new(),
            1,
            pos(2, 40.0),
        );
        let mut invalid = pos(3, 50.0);
        invalid.topology_generation = 8;
        motion(&mut actor, peer, &context, id, token, 1, invalid);
        assert!(frames(&events).is_empty());
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::Frame {
                session_id: id,
                token,
                target_display: "display".into(),
                sequence: 2,
                topology_generation: 7,
                events: vec![SharedInputEvent::Motion { x: 10.0, y: 10.0 }],
            },
        );
        assert!(!actor.leases.contains_key(&id));
        assert!(frames(&events).is_empty());
    }
    #[test]
    fn probe_timeout_and_health_failure_reacquire_only_new_reliable_sessions() {
        let (mut actor, peer, context, transport, _) = setup(true);
        enable(&transport, &context);
        transport.motion_ready.store(false, Ordering::Release);
        actor.request(peer, "display".into());
        assert!(actor.leases.is_empty());
        actor
            .motion_probe_started
            .insert(context.session_id, Instant::now() - Duration::from_secs(2));
        actor.request(peer, "display".into());
        let id = *actor.leases.keys().next().unwrap();
        assert!(!actor.leases[&id].hybrid.requested);
        actor.release(id, "test reconnect", false);
        transport.motion_ready.store(true, Ordering::Release);
        actor.request(peer, "display".into());
        let hybrid = *actor.leases.keys().next().unwrap();
        receive(
            &mut actor,
            peer,
            &context,
            InputShareMessage::GrantHybrid {
                session_id: hybrid,
                token: InputToken::new(),
                ttl_ms: 1500,
                motion_enabled: true,
            },
        );
        assert!(actor.leases[&hybrid].hybrid.enabled);
        transport.motion_ready.store(false, Ordering::Release);
        actor.tick_motion();
        assert!(actor.leases.is_empty());
        transport.motion_ready.store(true, Ordering::Release);
        actor.request(peer, "display".into());
        let new_id = *actor.leases.keys().next().unwrap();
        assert_ne!(hybrid, new_id);
        assert!(!actor.leases[&new_id].hybrid.requested);
        assert!(transport.motion_cancelled.lock().unwrap().contains(&hybrid));
    }
}
