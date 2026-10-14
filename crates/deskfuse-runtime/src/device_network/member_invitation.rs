//! Members initiate locally; the existing roster signer commits membership.
use super::*;

pub(super) struct RelayedInvitation {
    pub member: DeviceId,
    pub expires_at_unix_ms: u64,
    attempt: RequestId,
    target: DeviceId,
    reply: oneshot::Sender<Vec<u8>>,
}

impl Actor {
    pub(super) fn start_member_invitation(
        &mut self,
        roster: SignedNetworkRoster,
        remote: PeerHello,
        endpoint: Endpoint,
    ) -> Result<(), RuntimeError> {
        let owner = roster.roster.owner.clone();
        if self.blocked.contains(&owner.device_id)
            || !roster.roster.members.iter().any(|m| {
                m.device_id == self.identity.device_id()
                    && m.public_key == self.identity.verifying_key_bytes()
            })
        {
            return Err(invalid("本机尚未完成加入设备网络，请等待名单同步"));
        }
        let attempt = RequestId::new();
        let expires_at_unix_ms = now_ms().saturating_add(120_000);
        let target = NetworkMember {
            device_id: remote.device_id,
            public_key: remote.public_key,
            display_name: remote.display_name.clone(),
            capabilities: remote.capabilities,
            endpoint,
        };
        let payload = self.sign_network(NetworkOperation::InviteViaOwner {
            attempt,
            network_id: roster.roster.network_id,
            target,
            expires_at_unix_ms,
        })?;
        self.device_network.pending.insert(
            attempt,
            NetworkPairingPending {
                request_id: attempt,
                peer_id: remote.device_id,
                display_name: remote.display_name,
                incoming: false,
                expires_at_unix_ms,
                request: None,
                owner_name: owner.display_name.clone(),
                member_names: roster
                    .roster
                    .members
                    .iter()
                    .map(|m| m.display_name.clone())
                    .collect(),
            },
        );
        let transport = self.transport.clone();
        let tx = self.network_result_tx.clone();
        // Prefer an observed route: a persisted owner endpoint can become stale.
        let endpoint = self
            .peers
            .get(&owner.device_id)
            .and_then(|p| {
                self.policy
                    .select(&p.routes.values().cloned().collect::<Vec<_>>())
            })
            .map(|r| r.route.endpoint.clone())
            .unwrap_or_else(|| owner.endpoint.clone());
        self.device_network.tasks.push(task::spawn(async move {
            let result = transport
                .network_exchange(owner.device_id, endpoint, payload)
                .await;
            let _ = tx
                .send(NetworkResult::MemberInvitation {
                    attempt,
                    owner,
                    target: remote.device_id,
                    result,
                })
                .await;
        }));
        let _ = self.event_tx.send(RuntimeEvent::PairingRequestSent {
            peer_id: remote.device_id,
            request_id: attempt,
        });
        self.publish_view();
        Ok(())
    }

    pub(super) async fn receive_member_invitation(
        &mut self,
        request: NetworkTransportRequest,
        attempt: RequestId,
        network_id: RequestId,
        target: NetworkMember,
        expires_at_unix_ms: u64,
    ) -> Result<(), RuntimeError> {
        let member = request.remote.device_id;
        let authorized = self.device_network.roster.as_ref().is_some_and(|r| {
            r.roster.network_id == network_id
                && r.roster.owner.device_id == self.identity.device_id()
                && r.roster
                    .members
                    .iter()
                    .any(|m| m.device_id == member && m.public_key == request.remote.public_key)
        });
        if !authorized || self.blocked.contains(&member) {
            return Err(invalid("只有当前网络中的设备可以委托发起邀请"));
        }
        let relay = RelayedInvitation {
            member,
            expires_at_unix_ms,
            attempt,
            target: target.device_id,
            reply: request.reply,
        };
        let now = now_ms();
        if expires_at_unix_ms <= now
            || expires_at_unix_ms > now.saturating_add(125_000)
            || self.device_network.seen.contains_key(&attempt)
        {
            self.reply_to_member(relay, "邀请已过期或已处理，请重新发起");
            return Ok(());
        }
        self.device_network
            .seen
            .insert(attempt, now.saturating_add(300_000));
        if self.blocked.contains(&target.device_id) {
            self.reply_to_member(relay, "目标设备在网络协调设备上已被阻止，请先解除阻止");
            return Ok(());
        }
        if self
            .device_network
            .origins
            .forgotten
            .contains(&target.device_id)
        {
            self.reply_to_member(relay, "协调设备已忘记目标设备，请先在协调设备上重新配对");
            return Ok(());
        }
        if self.device_network.pending.len() >= 16
            || self
                .device_network
                .pending
                .values()
                .any(|p| p.peer_id == target.device_id)
            || self
                .device_network
                .incoming_rate
                .get(&member)
                .is_some_and(|t| now.saturating_sub(*t) < 5000)
        {
            self.reply_to_member(relay, "已有待处理的邀请或请求过于频繁，请稍后重试");
            return Ok(());
        }
        self.device_network.incoming_rate.insert(member, now);
        let route = RouteCandidate::new(
            target.endpoint,
            RouteSource::Manual,
            RouteHealth::Unknown,
            target.capabilities,
        );
        self.hints.insert(
            target.device_id,
            PeerHintState {
                display_name: target.display_name,
                capabilities: target.capabilities,
                routes: BTreeMap::from([(route.id, route)]),
            },
        );
        // The normal path independently verifies the target's signed Hello.
        let result = self.start_network_pairing(target.device_id).await;
        self.attach_relay(target.device_id, relay, result);
        Ok(())
    }

    pub(super) fn attach_relay(
        &mut self,
        target: DeviceId,
        relay: RelayedInvitation,
        result: Result<(), RuntimeError>,
    ) {
        if let Err(error) = result {
            self.reply_to_member(relay, &error.to_string());
        } else if let Some(pending) = self
            .device_network
            .pending
            .values_mut()
            .find(|p| p.peer_id == target && !p.incoming)
        {
            // Never replace another member's response channel after a redirect.
            if let std::collections::btree_map::Entry::Vacant(entry) =
                self.device_network.relayed.entry(pending.request_id)
            {
                pending.expires_at_unix_ms =
                    pending.expires_at_unix_ms.min(relay.expires_at_unix_ms);
                entry.insert(relay);
            } else {
                self.reply_to_member(relay, "目标已有待处理的邀请，请稍后重试");
            }
        } else if self
            .device_network
            .roster
            .as_ref()
            .is_some_and(|r| r.roster.members.iter().any(|m| m.device_id == target))
        {
            self.reply_to_member(relay, "accepted");
        } else {
            self.reply_to_member(relay, "邀请未能发起，请重试");
        }
    }

    pub(super) fn reply_to_member(&self, relay: RelayedInvitation, outcome: &str) {
        if let Ok(payload) = self.sign_network(NetworkOperation::InvitationResult {
            attempt: relay.attempt,
            target: relay.target,
            outcome: outcome.into(),
            roster: (outcome == "accepted")
                .then(|| self.device_network.roster.clone())
                .flatten(),
        }) {
            let _ = relay.reply.send(payload);
        }
    }

    pub(super) async fn complete_member_invitation(
        &mut self,
        attempt: RequestId,
        owner: NetworkMember,
        target: DeviceId,
        result: Result<(PeerHello, Vec<u8>), RuntimeError>,
    ) {
        let Some(pending) = self.device_network.pending.get(&attempt) else {
            return;
        };
        let expired = pending.expires_at_unix_ms <= now_ms();
        let completed = async {
            if self.blocked.contains(&owner.device_id) {
                return Err(RuntimeError::PeerBlocked(owner.device_id));
            }
            if expired {
                return Err(invalid("邀请已过期，请重新发起"));
            }
            let (hello, bytes) = result.map_err(|e| {
                invalid(format!(
                    "邀请协调失败，请确认 {} 在线且已升级后重试：{e}",
                    owner.display_name
                ))
            })?;
            if hello.device_id != owner.device_id || hello.public_key != owner.public_key {
                return Err(invalid("network invitation coordinator identity mismatch"));
            }
            let operation = SignedNetworkOperation::decode_verify(&bytes, &owner.public_key)
                .map_err(invalid)?
                .operation;
            let NetworkOperation::InvitationResult {
                attempt: received,
                target: received_target,
                outcome,
                roster,
            } = operation
            else {
                return Err(invalid("请将网络中的设备升级后重新发起邀请"));
            };
            if received != attempt || received_target != target {
                return Err(invalid("network invitation result mismatch"));
            }
            if outcome != "accepted" {
                return Err(invalid(match outcome.as_str() {
                    "rejected" => "接收方拒绝了邀请".into(),
                    "expired" => "邀请已过期，请重新发起".into(),
                    "failed" => "邀请未完成，请确认目标设备在线后重试".into(),
                    _ => outcome,
                }));
            }
            let roster = roster.ok_or_else(|| invalid("missing accepted network roster"))?;
            let member = roster
                .roster
                .members
                .iter()
                .find(|m| m.device_id == target)
                .cloned()
                .ok_or_else(|| invalid("accepted roster omits invited device"))?;
            // A periodic sync may already have delivered the migration roster.
            if self.device_network.roster.as_ref() != Some(&roster) {
                self.accept_network_roster(owner.device_id, roster).await?;
            } else {
                // Explicit re-pairing may restore a locally forgotten member
                // without changing the already verified network roster.
                for member in roster.roster.members {
                    self.admit_network_member(member).await?;
                }
            }
            self.admit_network_member_with_consent(member).await?;
            Ok::<_, RuntimeError>(())
        }
        .await;
        self.finish_network_pairing(
            attempt,
            target,
            if completed.is_ok() {
                "accepted"
            } else {
                "failed"
            },
        );
        if let Err(error) = completed {
            self.emit_error(error);
        }
    }
}
