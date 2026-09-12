//! A single-owner device network. Only its owner can change the signed roster.
use crate::{DeviceId, LocalIdentity};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unidesk_core::{Capabilities, Endpoint, RequestId};
const DOMAIN: &[u8] = b"uniDesk/device-network/v1\0";
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkMember {
    pub device_id: DeviceId,
    pub public_key: [u8; 32],
    pub display_name: String,
    pub capabilities: Capabilities,
    pub endpoint: Endpoint,
}
impl NetworkMember {
    pub fn validate(&self) -> Result<(), String> {
        if DeviceId::from_public_key(self.public_key).map_err(|e| e.to_string())? != self.device_id
            || self.display_name.trim().is_empty()
            || self.display_name.len() > 256
            || self.display_name.chars().any(char::is_control)
            || !self.capabilities.contains(Capabilities::DEVICE_NETWORK)
        {
            return Err("invalid device-network member identity or capability".into());
        }
        Endpoint::new(self.endpoint.host.clone(), self.endpoint.port).map_err(|e| e.to_string())?;
        Ok(())
    }
}
/// Durable delegation carried with the roster so offline members can migrate later.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkMergePermit {
    pub network_id: RequestId,
    pub epoch: u64,
    pub owner_public_key: [u8; 32],
    pub target_owner: DeviceId,
    pub attempt: RequestId,
    pub expires_at_unix_ms: u64,
    pub signature: Vec<u8>,
}
impl NetworkMergePermit {
    fn payload(&self) -> Result<Vec<u8>, String> {
        canonical(
            b"merge-permit\0",
            &(
                self.network_id,
                self.epoch,
                self.owner_public_key,
                self.target_owner,
                self.attempt,
                self.expires_at_unix_ms,
            ),
        )
    }
    pub fn signed(
        network_id: RequestId,
        epoch: u64,
        target_owner: DeviceId,
        attempt: RequestId,
        expires_at_unix_ms: u64,
        identity: &LocalIdentity,
    ) -> Result<Self, String> {
        let mut permit = Self {
            network_id,
            epoch,
            owner_public_key: identity.verifying_key_bytes(),
            target_owner,
            attempt,
            expires_at_unix_ms,
            signature: Vec::new(),
        };
        permit.signature = identity.sign(&permit.payload()?).as_ref().to_vec();
        Ok(permit)
    }
    pub fn verify(&self) -> Result<(), String> {
        LocalIdentity::verify_with_key(&self.owner_public_key, &self.payload()?, &self.signature)
            .map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkRoster {
    pub network_id: RequestId,
    pub owner: NetworkMember,
    pub epoch: u64,
    pub members: Vec<NetworkMember>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merge_permits: Vec<NetworkMergePermit>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SignedNetworkRoster {
    pub roster: NetworkRoster,
    pub signature: Vec<u8>,
}
fn canonical<T: Serialize>(kind: &[u8], value: &T) -> Result<Vec<u8>, String> {
    let mut bytes = DOMAIN.to_vec();
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(&serde_json::to_vec(value).map_err(|e| e.to_string())?);
    Ok(bytes)
}
impl SignedNetworkRoster {
    pub fn signed(roster: NetworkRoster, identity: &LocalIdentity) -> Result<Self, String> {
        if roster.owner.device_id != identity.device_id() {
            return Err("only the network owner may sign its roster".into());
        }
        let signature = identity
            .sign(&canonical(b"roster\0", &roster)?)
            .as_ref()
            .to_vec();
        let result = Self { roster, signature };
        result.verify()?;
        Ok(result)
    }
    pub fn authorizes_migration(&self, previous: &NetworkRoster) -> bool {
        if self.verify().is_err() {
            return false;
        }
        let mut owners = std::collections::BTreeSet::new();
        for permit in &self.roster.merge_permits {
            if permit.network_id == previous.network_id
                && permit.owner_public_key == previous.owner.public_key
                && permit.epoch >= previous.epoch
            {
                owners.insert(permit.target_owner);
            }
        }
        for _ in 0..self.roster.merge_permits.len() {
            if owners.contains(&self.roster.owner.device_id) {
                return true;
            }
            for permit in &self.roster.merge_permits {
                if DeviceId::from_public_key(permit.owner_public_key)
                    .is_ok_and(|id| owners.contains(&id))
                {
                    owners.insert(permit.target_owner);
                }
            }
        }
        owners.contains(&self.roster.owner.device_id)
    }
    pub fn verify(&self) -> Result<(), String> {
        self.roster.owner.validate()?;
        if self.roster.merge_permits.len() > 64 {
            return Err("network merge history limit".into());
        }
        for permit in &self.roster.merge_permits {
            permit.verify()?;
        }
        if self.roster.epoch == 0
            || self.roster.members.is_empty()
            || self.roster.members.len() > 32
        {
            return Err("network roster bounds".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for member in &self.roster.members {
            member.validate()?;
            if !ids.insert(member.device_id) {
                return Err("duplicate network member".into());
            }
        }
        if !self.roster.members.contains(&self.roster.owner) {
            return Err("network roster omits its owner".into());
        }
        LocalIdentity::verify_with_key(
            &self.roster.owner.public_key,
            &canonical(b"roster\0", &self.roster)?,
            &self.signature,
        )
        .map_err(|e| e.to_string())
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkOffer {
    pub attempt: RequestId,
    pub target: DeviceId,
    pub nonce: RequestId,
    pub expires_at_unix_ms: u64,
    pub roster: SignedNetworkRoster,
    pub merge_permit: NetworkMergePermit,
}
impl NetworkOffer {
    pub fn signed(
        roster: SignedNetworkRoster,
        target: DeviceId,
        expires_at_unix_ms: u64,
        identity: &LocalIdentity,
    ) -> Result<Self, String> {
        if roster.roster.owner.device_id != identity.device_id() {
            return Err("only owner may invite or merge".into());
        }
        let attempt = RequestId::new();
        let merge_permit = NetworkMergePermit::signed(
            roster.roster.network_id,
            roster.roster.epoch,
            target,
            attempt,
            expires_at_unix_ms,
            identity,
        )?;
        Ok(Self {
            attempt,
            target,
            nonce: RequestId::new(),
            expires_at_unix_ms,
            roster,
            merge_permit,
        })
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkDecision {
    pub attempt: RequestId,
    pub offer_hash: [u8; 32],
    pub target: DeviceId,
    pub accepted: bool,
    pub member: NetworkMember,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NetworkOperation {
    /// A current member asks the roster signer to coordinate an invitation.
    InviteViaOwner {
        attempt: RequestId,
        network_id: RequestId,
        target: NetworkMember,
        expires_at_unix_ms: u64,
    },
    InvitationResult {
        attempt: RequestId,
        target: DeviceId,
        outcome: String,
        roster: Option<SignedNetworkRoster>,
    },
    Offer(NetworkOffer),
    Decision(NetworkDecision),
    Joined {
        decision: NetworkDecision,
        roster: SignedNetworkRoster,
    },
    Redirect {
        attempt: RequestId,
        offer_hash: [u8; 32],
        roster: SignedNetworkRoster,
    },
    Roster(SignedNetworkRoster),
    Leave {
        network_id: RequestId,
    },
    Query {
        network_id: RequestId,
        epoch: u64,
    },
    Ack {
        network_id: RequestId,
        epoch: u64,
    },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SignedNetworkOperation {
    pub operation: NetworkOperation,
    pub signature: Vec<u8>,
}
impl SignedNetworkOperation {
    pub fn signed(operation: NetworkOperation, identity: &LocalIdentity) -> Result<Self, String> {
        let signature = identity
            .sign(&canonical(b"operation\0", &operation)?)
            .as_ref()
            .to_vec();
        Ok(Self {
            operation,
            signature,
        })
    }
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > 131072 {
            return Err("network payload bounds".into());
        }
        Ok(bytes)
    }
    pub fn decode_verify(bytes: &[u8], public_key: &[u8; 32]) -> Result<Self, String> {
        if bytes.len() > 131072 {
            return Err("network payload bounds".into());
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| "invalid network operation")?;
        LocalIdentity::verify_with_key(
            public_key,
            &canonical(b"operation\0", &value.operation)?,
            &value.signature,
        )
        .map_err(|e| e.to_string())?;
        match &value.operation {
            NetworkOperation::InviteViaOwner { target, .. } => target.validate()?,
            NetworkOperation::InvitationResult {
                roster: Some(r), ..
            } => r.verify()?,
            NetworkOperation::Offer(o) => {
                o.roster.verify()?;
                o.merge_permit.verify()?;
                let p = &o.merge_permit;
                if p.network_id != o.roster.roster.network_id
                    || p.epoch != o.roster.roster.epoch
                    || p.owner_public_key != o.roster.roster.owner.public_key
                    || p.target_owner != o.target
                    || p.attempt != o.attempt
                    || p.expires_at_unix_ms != o.expires_at_unix_ms
                {
                    return Err("merge permit does not match invitation".into());
                }
            }
            NetworkOperation::Roster(r) | NetworkOperation::Redirect { roster: r, .. } => {
                r.verify()?
            }
            NetworkOperation::Joined { decision, roster } => {
                decision.member.validate()?;
                roster.verify()?;
            }
            NetworkOperation::Decision(d) => d.member.validate()?,
            _ => {}
        }
        Ok(value)
    }
}
pub fn network_offer_hash(offer: &NetworkOffer) -> Result<[u8; 32], String> {
    Ok(Sha256::digest(canonical(b"offer\0", offer)?).into())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn member(identity: &LocalIdentity) -> NetworkMember {
        NetworkMember {
            device_id: identity.device_id(),
            public_key: identity.verifying_key_bytes(),
            display_name: "Device".into(),
            capabilities: Capabilities::DEVICE_NETWORK,
            endpoint: Endpoint::new("10.0.0.2", 1234).unwrap(),
        }
    }
    #[test]
    fn owner_roster_and_decision_signatures_bind_all_membership_fields() {
        let owner = LocalIdentity::from_bytes(&[91; 32]).unwrap();
        let other = LocalIdentity::from_bytes(&[92; 32]).unwrap();
        let owner_member = member(&owner);
        let roster = NetworkRoster {
            merge_permits: Vec::new(),
            network_id: RequestId::new(),
            owner: owner_member.clone(),
            epoch: 1,
            members: vec![owner_member],
        };
        assert!(SignedNetworkRoster::signed(roster.clone(), &other).is_err());
        let signed = SignedNetworkRoster::signed(roster, &owner).unwrap();
        let mut tampered = signed.clone();
        tampered.roster.members.push(member(&other));
        assert!(tampered.verify().is_err());
        tampered = signed.clone();
        tampered.roster.epoch = 2;
        assert!(tampered.verify().is_err());
        let offer = NetworkOffer::signed(signed, other.device_id(), 120000, &owner).unwrap();
        let decision = NetworkDecision {
            attempt: offer.attempt,
            offer_hash: network_offer_hash(&offer).unwrap(),
            target: owner.device_id(),
            accepted: true,
            member: member(&other),
            reason: None,
        };
        let operation =
            SignedNetworkOperation::signed(NetworkOperation::Decision(decision), &other).unwrap();
        let bytes = operation.encode().unwrap();
        assert!(
            SignedNetworkOperation::decode_verify(&bytes, &other.verifying_key_bytes()).is_ok()
        );
        assert!(
            SignedNetworkOperation::decode_verify(&bytes, &owner.verifying_key_bytes()).is_err()
        );
        let mut changed = operation;
        if let NetworkOperation::Decision(d) = &mut changed.operation {
            d.accepted = false;
        }
        assert!(
            SignedNetworkOperation::decode_verify(
                &changed.encode().unwrap(),
                &other.verifying_key_bytes()
            )
            .is_err()
        );
        let hash = network_offer_hash(&offer).unwrap();
        let mut changed = offer;
        changed.nonce = RequestId::new();
        assert_ne!(network_offer_hash(&changed).unwrap(), hash);
    }
    #[test]
    fn network_migration_requires_original_owner_signature_and_survives_merge_chains() {
        let a = LocalIdentity::from_bytes(&[121; 32]).unwrap();
        let c = LocalIdentity::from_bytes(&[122; 32]).unwrap();
        let e = LocalIdentity::from_bytes(&[123; 32]).unwrap();
        let original = NetworkRoster {
            network_id: RequestId::new(),
            owner: member(&a),
            epoch: 4,
            members: vec![member(&a)],
            merge_permits: Vec::new(),
        };
        let permit = NetworkMergePermit::signed(
            original.network_id,
            4,
            c.device_id(),
            RequestId::new(),
            120000,
            &a,
        )
        .unwrap();
        let mut merged = NetworkRoster {
            network_id: RequestId::new(),
            owner: member(&c),
            epoch: 2,
            members: vec![member(&a), member(&c)],
            merge_permits: Vec::new(),
        };
        assert!(
            !SignedNetworkRoster::signed(merged.clone(), &c)
                .unwrap()
                .authorizes_migration(&original)
        );
        merged.merge_permits.push(permit.clone());
        let signed = SignedNetworkRoster::signed(merged.clone(), &c).unwrap();
        assert!(signed.authorizes_migration(&original));
        let mut newer = original.clone();
        newer.epoch = 5;
        assert!(!signed.authorizes_migration(&newer));
        newer = original.clone();
        newer.network_id = RequestId::new();
        assert!(!signed.authorizes_migration(&newer));
        let mut forged = signed;
        forged.roster.merge_permits[0].target_owner = e.device_id();
        assert!(!forged.authorizes_migration(&original));
        assert!(SignedNetworkRoster::signed(forged.roster, &c).is_err());
        let next = NetworkMergePermit::signed(
            merged.network_id,
            merged.epoch,
            e.device_id(),
            RequestId::new(),
            240000,
            &c,
        )
        .unwrap();
        let final_roster = SignedNetworkRoster::signed(
            NetworkRoster {
                network_id: RequestId::new(),
                owner: member(&e),
                epoch: 1,
                members: vec![member(&a), member(&c), member(&e)],
                merge_permits: vec![permit, next],
            },
            &e,
        )
        .unwrap();
        assert!(final_roster.authorizes_migration(&original));
        assert!(final_roster.authorizes_migration(&merged));
        let bytes = serde_json::to_vec(&final_roster).unwrap();
        let restored: SignedNetworkRoster = serde_json::from_slice(&bytes).unwrap();
        assert!(restored.authorizes_migration(&original));
    }
}

/// Local provenance; network revocation never becomes a user's permanent block.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkTrustOrigins {
    pub introduced: std::collections::BTreeSet<DeviceId>,
    pub network_revoked: std::collections::BTreeSet<DeviceId>,
}
