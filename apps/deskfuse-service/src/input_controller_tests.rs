//! Windows-as-controller input IPC tests. These run only on Windows CI: they
//! cover the new desktop IPC surface (controller commands, deliveries, state)
//! and the helper-side conversions that must match the macOS coordinator.

#![cfg(windows)]
use crate::desktop_protocol as ipc;
use std::collections::BTreeMap;
use deskfuse_core::{
    Capabilities, DeviceId, DisplayDescriptor, DisplayId, DisplayTopology, InputSessionId,
    InputShareRole, InputSharingSettings, SharedInputEvent,
};

fn session() -> String {
    InputSessionId::new().to_string()
}

fn topology() -> DisplayTopology {
    DisplayTopology {
        generation: 1,
        displays: vec![DisplayDescriptor {
            id: DisplayId::from("win-1"),
            frame_x: 0.0,
            frame_y: 0.0,
            width: 1920,
            height: 1080,
            scale_factor: 1.0,
            rotation_degrees: 0,
            is_primary: true,
        }],
    }
}

/// Every new controller command must round-trip through the exact externally
/// tagged JSON shape the C# client sends.
#[test]
fn controller_commands_round_trip() {
    let commands = vec![
        ipc::Command::EnterInputControl {
            session_id: session(),
            target_display: "mac-display".into(),
            x: 12.5,
            y: -3.5,
        },
        ipc::Command::SendInputEvents {
            session_id: session(),
            target_display: "mac-display".into(),
            batch: 7,
            events: vec![
                SharedInputEvent::Motion { x: 10.0, y: 20.0 },
                SharedInputEvent::Key {
                    code: 4,
                    pressed: true,
                    modifiers: 0,
                    repeat: false,
                },
            ],
        },
        ipc::Command::LeaveInputControl {
            session_id: session(),
        },
        ipc::Command::ReleaseInputControl {
            session_id: session(),
        },
        ipc::Command::EnableInputControl {
            peer_id: DeviceId::from_bytes([9; 32]).to_hex(),
        },
        ipc::Command::DisableInputControl,
        ipc::Command::RestartInputControl {
            peer_id: DeviceId::from_bytes([9; 32]).to_hex(),
        },
        ipc::Command::MoveControlDevice {
            device_id: DeviceId::from_bytes([9; 32]).to_hex(),
            display_id: "mac-display".into(),
            x: 100.0,
            y: 50.0,
        },
    ];
    for command in commands {
        let request = ipc::Request {
            version: ipc::VERSION,
            request_id: 1,
            command,
        };
        let text = serde_json::to_string(&request).expect("serialize request");
        let parsed: ipc::Request = serde_json::from_str(&text).expect("parse request");
        assert_eq!(parsed.version, ipc::VERSION);
        assert_eq!(parsed.request_id, 1);
    }
    // The externally tagged names must match the C# sender exactly.
    let enter = ipc::Command::EnterInputControl {
        session_id: "s".into(),
        target_display: "d".into(),
        x: 0.0,
        y: 0.0,
    };
    let text = serde_json::to_string(&enter).expect("serialize");
    assert!(text.contains("\"EnterInputControl\""), "{text}");
    let send = ipc::Command::SendInputEvents {
        session_id: "s".into(),
        target_display: "d".into(),
        batch: 1,
        events: Vec::new(),
    };
    let text = serde_json::to_string(&send).expect("serialize");
    assert!(text.contains("\"SendInputEvents\""), "{text}");
}

/// Controller deliveries must carry their own kind markers so the helper can
/// branch without guessing from string content.
#[test]
fn controller_deliveries_serialize_and_classify() {
    let granted = ipc::InputDelivery::ControlGranted {
        sequence: 3,
        session_id: session(),
        lease_ms: 1_500,
    };
    let denied = ipc::InputDelivery::ControlDenied {
        sequence: 4,
        reason: "peer paused".into(),
    };
    let released = ipc::InputDelivery::ControlReleased {
        sequence: 5,
        session_id: session(),
        reason: "left display".into(),
    };
    for delivery in [&granted, &denied, &released] {
        assert!(delivery.is_controller_event());
        let text = serde_json::to_string(delivery).expect("serialize");
        let parsed: ipc::InputDelivery = serde_json::from_str(&text).expect("parse delivery");
        assert_eq!(parsed.sequence(), delivery.sequence());
    }
    let lease = ipc::InputDelivery::Lease {
        sequence: 1,
        session_id: session(),
        lease_revision: 1,
    };
    assert!(!lease.is_controller_event());
}

/// The expanded input state must expose the layout-relevant fields the helper
/// and the GUI parse, with the exact field names the C# client reads.
#[test]
fn input_state_carries_layout_and_control_views() {
    let peer = DeviceId::from_bytes([7; 32]);
    let state = ipc::InputState {
        security: ipc::InputSecurityState::default(),
        helper_connected: true,
        managed: false,
        status: "standby".into(),
        reason: None,
        controller_id: None,
        display_count: 1,
        local_device_id: DeviceId::from_bytes([1; 32]).to_hex(),
        settings: Some(InputSharingSettings {
            enabled: true,
            role: InputShareRole::Controller,
            participants: vec![peer],
            ..InputSharingSettings::default()
        }),
        controlling: Some(ipc::ControlView {
            session_id: session(),
            peer_id: peer.to_hex(),
            target_display: "mac-display".into(),
        }),
        activations: vec![ipc::ActivationView {
            peer_id: peer.to_hex(),
            status: "Ready".into(),
            reason: None,
        }],
        remote_topologies: vec![ipc::RemoteTopologyView {
            peer_id: peer.to_hex(),
            topology: topology(),
        }],
        links: vec![ipc::LinkView {
            peer_id: peer.to_hex(),
            ready: true,
            control_session: Some(session()),
            lease_revision: 3,
            motion_mode: Some("hybrid".into()),
        }],
    };
    let text = serde_json::to_string(&state).expect("serialize state");
    for field in [
        "\"local_device_id\"",
        "\"settings\"",
        "\"controlling\"",
        "\"activations\"",
        "\"remote_topologies\"",
        "\"links\"",
    ] {
        assert!(text.contains(field), "missing {field} in {text}");
    }
    let parsed: ipc::InputState = serde_json::from_str(&text).expect("parse state");
    assert_eq!(parsed.remote_topologies.len(), 1);
    assert_eq!(parsed.links[0].lease_revision, 3);
    assert!(parsed.controlling.is_some());
}

/// The settings toggle must preserve saved shortcuts and only flip role and
/// participants, mirroring the macOS one-click enable/disable cycle.
#[test]
fn controller_settings_toggle_preserves_saved_configuration() {
    use crate::native::desktop::controller_settings;
    let peer = DeviceId::from_bytes([7; 32]);
    let other = DeviceId::from_bytes([8; 32]);
    let saved = InputSharingSettings {
        enabled: false,
        role: InputShareRole::Disabled,
        participants: vec![other],
        corner_size: 42,
        ..InputSharingSettings::default()
    };
    let enabled = controller_settings(saved.clone(), Some(peer));
    assert!(enabled.enabled);
    assert_eq!(enabled.role, InputShareRole::Controller);
    assert!(enabled.participants.contains(&peer));
    assert!(enabled.participants.contains(&other));
    assert_eq!(enabled.corner_size, 42);
    assert!(enabled.validate().is_ok());
    let disabled = controller_settings(enabled, None);
    assert!(!disabled.enabled);
    assert_eq!(disabled.role, InputShareRole::Disabled);
    // Participants survive so the next enable keeps the saved pairing.
    assert!(disabled.participants.contains(&peer));
    assert_eq!(disabled.corner_size, 42);
}

/// The helper-side hotkey conversion must produce the same chords the macOS
/// coordinator derives from the shared settings.
#[test]
fn helper_hotkey_bindings_match_settings() {
    use crate::native::input_helper::hotkey_bindings;
    let settings = InputSharingSettings::default();
    let bindings = hotkey_bindings(&settings);
    let defaults = InputSharingSettings::default().shortcuts;
    assert_eq!(bindings.directional[0].code, defaults.left.key_code);
    assert_eq!(bindings.directional[0].modifiers, defaults.left.modifiers);
    assert_eq!(bindings.lock.code, defaults.lock.key_code);
}

/// Session identifiers parse exactly like the receiver path.
#[test]
fn helper_parses_session_ids() {
    use crate::native::input_helper::parse_session;
    let id = InputSessionId::new();
    assert_eq!(parse_session(&id.to_string()).expect("parse"), id);
    assert!(parse_session("not-a-uuid").is_err());
}

/// The input runtime snapshot types the service mirrors stay compatible with
/// the runtime's own activation bookkeeping (map keyed by device).
#[test]
fn activation_views_match_runtime_status_names() {
    let mut map = BTreeMap::new();
    map.insert(
        DeviceId::from_bytes([7; 32]),
        deskfuse_core::InputActivationState {
            activation_id: deskfuse_core::InputActivationId::new(),
            status: deskfuse_core::InputActivationStatus::Ready,
            reason: None,
        },
    );
    let json = serde_json::to_string(&map).expect("serialize activations");
    assert!(json.contains("\"Ready\""), "{json}");
    let _ = Capabilities::INPUT_SHARING;
}
