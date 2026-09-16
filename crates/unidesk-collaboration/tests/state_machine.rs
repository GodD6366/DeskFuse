use std::sync::Arc;
use unidesk_collaboration::{CollaborationCommand, CollaborationCore, MemoryTransport};
use unidesk_core::DeviceId;
use unidesk_platform::ports::HostPorts;

fn device(byte: u8) -> DeviceId {
    DeviceId::from_public_key([byte; 32]).expect("32-byte public key")
}

#[tokio::test]
async fn accept_device_creates_a_connected_session() {
    let temp = tempfile::tempdir().expect("temp");
    let ports = HostPorts::memory(temp.path());
    let transport = Arc::new(MemoryTransport::new());
    let core = CollaborationCore::new_desktop(ports, transport, temp.path());
    let peer = device(5);
    core.discover(unidesk_collaboration::DeviceInfo {
        id: peer,
        name: "win-b".into(),
    })
    .await;
    core.submit(CollaborationCommand::AcceptDevice { device_id: peer })
        .await
        .expect("accept");
    let snapshot = core.snapshot().await;
    assert!(
        snapshot
            .devices
            .iter()
            .any(|device| device.id == peer && device.trusted)
    );
    assert!(
        snapshot
            .active_connections
            .iter()
            .any(|connection| connection.device_id == peer)
    );
}
