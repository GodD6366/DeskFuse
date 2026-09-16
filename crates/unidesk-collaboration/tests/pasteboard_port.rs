use std::sync::Arc;
use unidesk_collaboration::{CollaborationCore, MemoryTransport, RemoteContent};
use unidesk_core::DeviceId;
use unidesk_platform::{
    MemoryPasteboard,
    ports::{ClipboardCapturePort, ContentData, HostPorts, PasteboardPort},
};

fn device(byte: u8) -> DeviceId {
    DeviceId::from_public_key([byte; 32]).expect("32-byte public key")
}

#[tokio::test]
async fn production_pasteboard_adapter_applies_and_does_not_echo() {
    let directory = std::env::temp_dir().join("unidesk-pasteboard-port");
    let port = Arc::new(PasteboardPort::new(MemoryPasteboard::new()));
    let ports = HostPorts {
        clipboard_capture: port.clone(),
        clipboard_apply: port.clone(),
        ..HostPorts::memory(&directory)
    };
    let transport = Arc::new(MemoryTransport::new());
    let core = CollaborationCore::new_desktop(ports, transport.clone(), directory);
    core.clipboard()
        .on_remote_clipboard(
            device(1),
            RemoteContent {
                generation: 1,
                operation_id: unidesk_collaboration::OperationId::new(),
                data: ContentData::text("from peer"),
                captured_at_ms: 5,
            },
        )
        .await
        .expect("apply remote");
    let captured = port.capture().await.expect("capture");
    assert_eq!(captured.data.as_text(), Some("from peer"));
    assert_eq!(
        captured.source,
        unidesk_platform::ports::CaptureSource::Remote
    );
    transport.connect(device(2));
    core.clipboard()
        .on_local_capture(captured)
        .await
        .expect("ignore echo");
    assert!(transport.sent().is_empty());
}
