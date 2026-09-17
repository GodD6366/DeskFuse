use std::sync::Arc;
use std::time::Duration;
use unidesk_collaboration::{CollaborationCore, CollaborationError, MemoryTransport};
use unidesk_core::{DeviceId, SharedInputEvent};
use unidesk_platform::ports::HostPorts;

fn device(byte: u8) -> DeviceId {
    DeviceId::from_public_key([byte; 32]).expect("32-byte public key")
}

#[tokio::test]
async fn lease_expiry_releases_keys_and_rejects_input() {
    tokio::time::pause();
    let temp = tempfile::tempdir().expect("temp");
    let ports = HostPorts::memory(temp.path());
    let transport = Arc::new(MemoryTransport::new());
    let core = CollaborationCore::new_input_only(ports, transport, temp.path());
    let controller = device(4);
    core.input()
        .grant_lease(controller, Duration::from_secs(10))
        .await
        .expect("grant");
    assert!(core.input().has_active_lease().await);
    core.input()
        .handle_input(SharedInputEvent::Key {
            code: 0,
            pressed: true,
            modifiers: 0,
            repeat: false,
        })
        .await
        .expect("key down");
    assert_eq!(core.input().pressed_keys().await.len(), 1);

    tokio::task::yield_now().await;
    tokio::time::sleep(Duration::from_secs(11)).await;

    assert!(!core.input().has_active_lease().await);
    assert!(core.input().pressed_keys().await.is_empty());
    let result = core
        .input()
        .handle_input(SharedInputEvent::Key {
            code: 0,
            pressed: true,
            modifiers: 0,
            repeat: false,
        })
        .await;
    assert!(matches!(result, Err(CollaborationError::LeaseExpired)));
}

#[tokio::test]
async fn input_only_core_rejects_clipboard_commands() {
    let temp = tempfile::tempdir().expect("temp");
    let ports = HostPorts::memory(temp.path());
    let transport = Arc::new(MemoryTransport::new());
    let core = CollaborationCore::new_input_only(ports, transport, temp.path());
    let result = core
        .submit(unidesk_collaboration::CollaborationCommand::CancelClipboard)
        .await;
    assert!(result.is_ok());
    let result = core
        .submit(
            unidesk_collaboration::CollaborationCommand::PublishClipboard {
                content: unidesk_collaboration::ContentHandle::from_data(
                    &unidesk_platform::ports::ContentData::text("unused"),
                ),
            },
        )
        .await;
    assert!(result.is_err());
}
