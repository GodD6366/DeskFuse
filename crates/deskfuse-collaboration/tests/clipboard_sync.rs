use deskfuse_collaboration::{CollaborationCore, MemoryTransport, RemoteContent};
use deskfuse_core::DeviceId;
use deskfuse_platform::ports::{CaptureSource, HostPorts, MemoryClipboard};
use std::sync::Arc;
use std::time::Duration;

fn device(byte: u8) -> DeviceId {
    DeviceId::from_public_key([byte; 32]).expect("32-byte public key")
}

#[tokio::test]
async fn remote_write_is_not_echoed() {
    let ports = HostPorts::memory(std::env::temp_dir().join("deskfuse-collab-echo"));
    let transport = Arc::new(MemoryTransport::new());
    let peer = device(2);
    transport.connect(peer);
    let core = CollaborationCore::new_desktop(ports, transport.clone(), std::env::temp_dir());
    let clipboard = Arc::new(MemoryClipboard::new());
    let captured = clipboard
        .capture_text("remote body")
        .expect("capture")
        .clone();
    let mut remote_marked = captured;
    remote_marked.source = CaptureSource::Remote;
    core.clipboard()
        .on_local_capture(remote_marked)
        .await
        .expect("ignore remote");
    assert!(transport.sent().is_empty());
}

#[tokio::test]
async fn duplicate_generation_is_not_applied_twice() {
    let clipboard = Arc::new(MemoryClipboard::new());
    let ports = HostPorts {
        clipboard_capture: clipboard.clone(),
        clipboard_apply: clipboard.clone(),
        ..HostPorts::memory(std::env::temp_dir().join("deskfuse-collab-dup"))
    };
    let transport = Arc::new(MemoryTransport::new());
    let core = CollaborationCore::new_desktop(
        ports,
        transport,
        std::env::temp_dir().join("deskfuse-collab-dup"),
    );
    let from = device(1);
    let content = RemoteContent {
        generation: 4,
        operation_id: deskfuse_collaboration::OperationId::new(),
        data: deskfuse_platform::ports::ContentData::text("Hello from Mac"),
        captured_at_ms: 10,
    };
    core.clipboard()
        .on_remote_clipboard(from, content.clone())
        .await
        .expect("first apply");
    core.clipboard()
        .on_remote_clipboard(from, content)
        .await
        .expect("duplicate ignored");
    assert_eq!(
        clipboard.current_text().expect("text").as_deref(),
        Some("Hello from Mac")
    );
    assert_eq!(clipboard.apply_count().expect("count"), 1);
}

#[tokio::test]
async fn local_user_copy_supersedes_pending_remote() {
    let clipboard = Arc::new(MemoryClipboard::new());
    let ports = HostPorts {
        clipboard_capture: clipboard.clone(),
        clipboard_apply: clipboard.clone(),
        ..HostPorts::memory(std::env::temp_dir().join("deskfuse-collab-super"))
    };
    let transport = Arc::new(MemoryTransport::new());
    let peer = device(9);
    transport.connect(peer);
    let core = CollaborationCore::new_desktop(
        ports,
        transport.clone(),
        std::env::temp_dir().join("deskfuse-collab-super"),
    );
    let local = clipboard.capture_text("User typed").expect("local copy");
    core.clipboard()
        .on_local_capture(local)
        .await
        .expect("publish local");
    let delayed = RemoteContent {
        generation: 1,
        operation_id: deskfuse_collaboration::OperationId::new(),
        data: deskfuse_platform::ports::ContentData::text("Delayed Mac clipboard"),
        captured_at_ms: 0,
    };
    core.clipboard()
        .on_remote_clipboard(device(3), delayed)
        .await
        .expect("remote after local");
    assert_eq!(
        clipboard.current_text().expect("text").as_deref(),
        Some("User typed")
    );
    let _ = Duration::from_millis(0);
}
