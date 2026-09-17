use std::sync::Arc;
use unidesk_collaboration::CollaborationCore;
use unidesk_collaboration::MemoryTransport;
use unidesk_core::DeviceId;
use unidesk_platform::ports::HostPorts;

fn device(byte: u8) -> DeviceId {
    DeviceId::from_public_key([byte; 32]).expect("32-byte public key")
}

#[tokio::test]
async fn send_files_notifies_peer_and_completes() {
    let temp = tempfile::tempdir().expect("temp");
    std::fs::write(temp.path().join("note.txt"), b"hello").expect("write");
    let ports = HostPorts::memory(temp.path().join("host"));
    let transport = Arc::new(MemoryTransport::new());
    let peer = device(7);
    transport.connect(peer);
    let core = CollaborationCore::new_desktop(ports, transport.clone(), temp.path());
    let transfer_id = core
        .files()
        .send_files(vec![temp.path().join("note.txt")], peer)
        .await
        .expect("send");
    let sent = transport.sent();
    assert!(sent.iter().any(|(to, _)| *to == peer));
    let snapshot = core.files().snapshot().await;
    assert!(snapshot.iter().any(|item| item.transfer_id == transfer_id));
}

#[tokio::test]
async fn cancel_unknown_transfer_fails() {
    let temp = tempfile::tempdir().expect("temp");
    let ports = HostPorts::memory(temp.path());
    let transport = Arc::new(MemoryTransport::new());
    let core = CollaborationCore::new_desktop(ports, transport, temp.path());
    let result = core
        .files()
        .cancel(unidesk_collaboration::TransferId::new())
        .await;
    assert!(result.is_err());
}
