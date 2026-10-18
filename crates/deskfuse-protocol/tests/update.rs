use deskfuse_protocol::update::*;
use deskfuse_protocol::{Envelope, Message, decode_frame, encode_frame};
fn wrap(message: Message) -> Envelope {
    Envelope::new([1; 16], message)
}
#[test]
fn all_update_messages_roundtrip_independently_of_clipboard() {
    let messages = vec![
        Message::ReleaseInventory(ReleaseInventory {
            installed_version: "0.1.0".into(),
            installed_build: 1,
            installed_release_id: vec![2; 16],
            target: 1,
            signed_manifests: vec![b"signed-manifest".to_vec()],
            github_artifacts: vec![b"github-artifact".to_vec()],
        }),
        Message::UpdateRequest(UpdateRequest {
            offset: 0,
            transaction_id: vec![1; 16],
            release_id: vec![2; 16],
            target: 2,
            sha256: vec![3; 32],
        }),
        Message::UpdateChunk(UpdateChunk {
            transaction_id: vec![1; 16],
            offset: 0,
            data: vec![8; 64],
        }),
        Message::UpdateCancel(UpdateCancel {
            transaction_id: vec![1; 16],
            reason: 1,
        }),
        Message::UpdateStatus(UpdateStatus {
            transaction_id: vec![1; 16],
            phase: 2,
            received_bytes: 1,
            total_bytes: 64,
        }),
        Message::UpdateReceipt(UpdateReceipt {
            transaction_id: vec![1; 16],
            release_id: vec![2; 16],
            installed_version: "0.1.0".into(),
            installed_build: 2,
            target: 1,
            sha256: vec![3; 32],
        }),
        Message::UpdateCommand(UpdateCommand { action: 1 }),
        Message::UpdateCommandAck(UpdateCommandAck { status: 1 }),
    ];
    for message in messages {
        let envelope = wrap(message);
        let bytes = encode_frame(&envelope).unwrap();
        assert_eq!(decode_frame(&bytes).unwrap(), envelope);
    }
}

#[test]
fn update_trigger_rejects_unknown_actions_and_statuses() {
    assert!(
        wrap(Message::UpdateCommand(UpdateCommand { action: 0 }))
            .validate()
            .is_err()
    );
    assert!(
        wrap(Message::UpdateCommandAck(UpdateCommandAck { status: 0 }))
            .validate()
            .is_err()
    );
    assert!(
        wrap(Message::UpdateCommandAck(UpdateCommandAck { status: 4 }))
            .validate()
            .is_err()
    );
}
#[test]
fn update_chunks_are_bounded_and_redacted() {
    let mut chunk = UpdateChunk {
        transaction_id: vec![1; 16],
        offset: 0,
        data: b"PRIVATE_PACKAGE_BODY".to_vec(),
    };
    assert!(!format!("{chunk:?}").contains("PRIVATE_PACKAGE_BODY"));
    chunk.offset = u64::MAX;
    assert!(
        wrap(Message::UpdateChunk(chunk.clone()))
            .validate()
            .is_err()
    );
    chunk.offset = 0;
    chunk.data = vec![0; MAX_UPDATE_CHUNK + 1];
    assert!(wrap(Message::UpdateChunk(chunk)).validate().is_err());
}
#[test]
fn reject_unsupported_targets_and_unbounded_inventory() {
    let mut v = ReleaseInventory {
        installed_version: "0.1.0".into(),
        installed_build: 0,
        installed_release_id: vec![],
        target: 3,
        signed_manifests: vec![],
        github_artifacts: vec![],
    };
    assert!(
        wrap(Message::ReleaseInventory(v.clone()))
            .validate()
            .is_err()
    );
    v.target = 1;
    v.signed_manifests = vec![vec![1]; MAX_RELEASE_INVENTORY + 1];
    assert!(
        wrap(Message::ReleaseInventory(v.clone()))
            .validate()
            .is_err()
    );
    v.signed_manifests = vec![];
    v.github_artifacts = vec![vec![1]; MAX_RELEASE_INVENTORY + 1];
    assert!(wrap(Message::ReleaseInventory(v)).validate().is_err());
}
