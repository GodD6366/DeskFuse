#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]

//! DeskFuse protocol version 2.
//!
//! This crate owns the stable, prost-based wire representation.  It contains
//! no generated source or build-time `protoc` dependency: all prost derives
//! are on hand-written message declarations in [`wire`].

pub mod clipboard_sync;
mod codec;
pub mod core;
mod error;
mod identity;
mod motion;
mod relative_motion;
pub mod update;
pub mod wire;

pub use codec::{FrameCodec, decode_frame, encode_frame};
pub use core::{
    clipboard_from_wire, clipboard_to_wire, core_device_id_matches_public_key,
    core_pairing_request_from_envelope, core_pairing_request_from_pairing_envelope,
    current_core_protocol_version, device_id_from_wire, device_id_to_wire, display_info_from_wire,
    display_info_to_wire, envelope_from_core_pairing_request, hello_from_core_pairing_request,
    input_batch_from_wire, input_batch_to_wire, pairing_request_from_core,
    peer_info_from_core_snapshot, protocol_version_from_wire, protocol_version_to_wire,
    request_id_from_wire, request_id_to_wire,
};
pub use error::ProtocolError;
pub use identity::{
    ED25519_PUBLIC_KEY_SIZE, ED25519_SIGNATURE_SIZE, HELLO_NONCE_SIZE, canonical_hello_payload,
    device_id_for_public_key, sign_hello, sign_hello_in_place, verify_hello,
};
pub use motion::{MAX_MOTION_DATAGRAM_SIZE, decode_motion_datagram, encode_motion_datagram};
pub use relative_motion::{decode_relative_motion_datagram, encode_relative_motion_datagram};
pub use wire::envelope::Message;
pub use wire::{
    ClipboardUpdate, DisplayInfo, Envelope, Error, Goodbye, Hello, HelloAck, InputEndpoints,
    InputEvent, InputEventBatch, InputSharing, MicrophoneOpen, MicrophoneShortcutOpen,
    NetworkControl, PairingDecision, PairingRequest, PeerInfo, Ping, Pong, RemoteApplicationOpen,
    SessionProof, WorkflowSync, envelope, input_event,
};

/// ALPN protocol identifier.
pub const ALPN: &[u8] = b"deskfuse/2";
/// Major protocol version.
pub const PROTOCOL_MAJOR: u32 = 2;
/// Minor protocol version.  Minor versions are wire-compatible unless a
/// message-specific validator says otherwise.
pub const PROTOCOL_MINOR: u32 = 0;
/// Maximum protobuf payload size, not including the four-byte frame prefix.
pub const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;
/// Wire-level request ID size.
pub const REQUEST_ID_SIZE: usize = 16;
/// Wire-level device ID size.
pub const DEVICE_ID_SIZE: usize = 32;

impl Envelope {
    /// Construct an envelope with the current protocol version.
    #[must_use]
    pub fn new(request_id: impl AsRef<[u8]>, message: Message) -> Self {
        Self {
            major: PROTOCOL_MAJOR,
            minor: PROTOCOL_MINOR,
            request_id: request_id.as_ref().to_vec(),
            message: Some(message),
        }
    }

    /// Validate version, request id, payload shape, and signed identity
    /// messages.  Decoders call this before returning an envelope to callers.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.major != PROTOCOL_MAJOR {
            return Err(ProtocolError::IncompatibleMajor {
                received: self.major,
                supported: PROTOCOL_MAJOR,
            });
        }
        if self.request_id.len() != REQUEST_ID_SIZE {
            return Err(ProtocolError::InvalidRequestId {
                expected: REQUEST_ID_SIZE,
                actual: self.request_id.len(),
            });
        }

        let Some(message) = self.message.as_ref() else {
            return Err(ProtocolError::UnknownPayload);
        };
        validate_message(message)
    }
}

fn validate_message(message: &Message) -> Result<(), ProtocolError> {
    match message {
        Message::MicrophoneShortcutOpen(open) => {
            if open.version == 1 && open.key_code <= 127 && open.modifiers & !0x9E0000 == 0 {
                Ok(())
            } else {
                Err(ProtocolError::InvalidPayload("microphone shortcut"))
            }
        }

        Message::MicrophoneOpen(open) => {
            if open.version == 1 && open.status <= 5 {
                Ok(())
            } else {
                Err(ProtocolError::InvalidPayload("microphone version/status"))
            }
        }
        Message::RemoteApplicationOpen(open) => {
            if open.version == 1 {
                Ok(())
            } else {
                Err(ProtocolError::InvalidPayload("remote application version"))
            }
        }
        Message::ReleaseInventory(_)
        | Message::UpdateRequest(_)
        | Message::UpdateChunk(_)
        | Message::UpdateCancel(_)
        | Message::UpdateStatus(_)
        | Message::UpdateReceipt(_)
        | Message::UpdateCommand(_)
        | Message::UpdateCommandAck(_) => update::validate(message),
        Message::SessionProof(proof) => {
            if proof.signature.len() == 64 {
                Ok(())
            } else {
                Err(ProtocolError::InvalidPayload(
                    "invalid session proof signature",
                ))
            }
        }
        Message::NetworkControl(control) => {
            if control.payload.is_empty() || control.payload.len() > 131072 {
                Err(ProtocolError::InvalidPayload("network payload bounds"))
            } else {
                Ok(())
            }
        }
        Message::InputSharing(input) => input.to_core().map(|_| ()),
        Message::InputEndpoints(endpoints) => {
            if endpoints.addresses.len() > 32
                || endpoints
                    .addresses
                    .iter()
                    .any(|a| a.parse::<std::net::SocketAddr>().is_err())
            {
                Err(ProtocolError::InvalidPayload("invalid input endpoint list"))
            } else {
                Ok(())
            }
        }
        Message::Hello(hello) => {
            crate::identity::validate_hello_identity_fields(hello, true)?;
            verify_hello(hello)
        }
        Message::HelloAck(ack) => {
            validate_device_id(&ack.device_id)?;
            if !ack.nonce.is_empty() && ack.nonce.len() != HELLO_NONCE_SIZE {
                return Err(ProtocolError::InvalidNonce {
                    expected: HELLO_NONCE_SIZE,
                    actual: ack.nonce.len(),
                });
            }
            if !ack.signature.is_empty() && ack.signature.len() != ED25519_SIGNATURE_SIZE {
                return Err(ProtocolError::InvalidSignatureLength {
                    expected: ED25519_SIGNATURE_SIZE,
                    actual: ack.signature.len(),
                });
            }
            Ok(())
        }
        Message::Ping(ping) => validate_optional_nonce(&ping.nonce),
        Message::Pong(pong) => validate_optional_nonce(&pong.nonce),
        Message::PeerInfo(info) => {
            validate_device_id(&info.device_id)?;
            validate_public_key_and_identity(&info.device_id, &info.public_key)?;
            validate_display_name(&info.display_name)?;
            Ok(())
        }
        Message::PairingRequest(request) => {
            let hello = Hello {
                device_id: request.device_id.clone(),
                display_name: request.display_name.clone(),
                public_key: request.public_key.clone(),
                nonce: request.nonce.clone(),
                signature: request.signature.clone(),
                capabilities: request.capabilities,
            };
            crate::identity::validate_hello_identity_fields(&hello, true)?;
            verify_hello(&hello)
        }
        Message::PairingDecision(decision) => {
            validate_device_id(&decision.device_id)?;
            validate_optional_nonce(&decision.nonce)?;
            if !decision.signature.is_empty() && decision.signature.len() != ED25519_SIGNATURE_SIZE
            {
                return Err(ProtocolError::InvalidSignatureLength {
                    expected: ED25519_SIGNATURE_SIZE,
                    actual: decision.signature.len(),
                });
            }
            if decision.reason.len() > 1024 {
                return Err(ProtocolError::InvalidPayload(
                    "pairing decision reason is too long",
                ));
            }
            Ok(())
        }
        Message::Error(error) => {
            if error.message.len() > 4096 {
                return Err(ProtocolError::InvalidPayload("error message is too long"));
            }
            Ok(())
        }
        Message::Goodbye(goodbye) => {
            if goodbye.reason.len() > 1024 {
                return Err(ProtocolError::InvalidPayload("goodbye reason is too long"));
            }
            Ok(())
        }
        Message::ClipboardUpdate(update) => {
            if update.mime.is_empty() || update.mime.len() > 256 {
                return Err(ProtocolError::InvalidPayload(
                    "clipboard mime type must be 1..=256 bytes",
                ));
            }
            if update.payload.len() > MAX_FRAME_SIZE - 256 {
                return Err(ProtocolError::InvalidPayload(
                    "clipboard payload is too large",
                ));
            }
            Ok(())
        }
        Message::InputEventBatch(batch) => {
            if batch.events.is_empty() || batch.events.len() > 512 {
                return Err(ProtocolError::InvalidPayload(
                    "input batch must contain 1..=512 events",
                ));
            }
            for event in &batch.events {
                validate_input_event(event)?;
            }
            Ok(())
        }
        Message::WorkflowSync(sync) => {
            if sync.payload.is_empty() || sync.payload.len() > 128 * 1024 {
                Err(ProtocolError::InvalidPayload("workflow frame bounds"))
            } else {
                Ok(())
            }
        }
        Message::DisplayInfo(display) => {
            if display.width == 0
                || display.height == 0
                || display.width > 32_768
                || display.height > 32_768
            {
                return Err(ProtocolError::InvalidPayload(
                    "display dimensions must be within 1..=32768",
                ));
            }
            Ok(())
        }
    }
}

fn validate_input_event(event: &InputEvent) -> Result<(), ProtocolError> {
    use input_event::Kind;
    match event.kind.as_ref() {
        Some(Kind::MouseMove(motion)) => {
            if !motion.x.is_finite()
                || !motion.y.is_finite()
                || !(0.0..=1.0).contains(&motion.x)
                || !(0.0..=1.0).contains(&motion.y)
            {
                return Err(ProtocolError::InvalidPayload(
                    "mouse coordinates must be finite and normalized to 0.0..=1.0",
                ));
            }
            Ok(())
        }
        Some(Kind::MouseButton(button)) => {
            if button.button > 31 {
                return Err(ProtocolError::InvalidPayload(
                    "mouse button number is too large",
                ));
            }
            Ok(())
        }
        Some(Kind::Key(key)) => {
            if key.code > u16::MAX as u32 {
                return Err(ProtocolError::InvalidPayload("key code is too large"));
            }
            Ok(())
        }
        Some(Kind::Scroll(scroll)) => {
            if !scroll.delta_x.is_finite() || !scroll.delta_y.is_finite() {
                return Err(ProtocolError::InvalidPayload(
                    "scroll deltas must be finite",
                ));
            }
            Ok(())
        }
        None => Err(ProtocolError::UnknownPayload),
    }
}

fn validate_device_id(device_id: &[u8]) -> Result<(), ProtocolError> {
    if device_id.len() != DEVICE_ID_SIZE {
        return Err(ProtocolError::InvalidDeviceId {
            expected: DEVICE_ID_SIZE,
            actual: device_id.len(),
        });
    }
    Ok(())
}

fn validate_public_key_and_identity(
    device_id: &[u8],
    public_key: &[u8],
) -> Result<(), ProtocolError> {
    if public_key.len() != ED25519_PUBLIC_KEY_SIZE {
        return Err(ProtocolError::InvalidPublicKey);
    }
    if device_id != device_id_for_public_key(public_key) {
        return Err(ProtocolError::IdentityMismatch);
    }
    Ok(())
}

fn validate_display_name(display_name: &str) -> Result<(), ProtocolError> {
    if display_name.is_empty() || display_name.chars().count() > 256 {
        return Err(ProtocolError::InvalidDisplayName);
    }
    Ok(())
}

fn validate_optional_nonce(nonce: &[u8]) -> Result<(), ProtocolError> {
    if !nonce.is_empty() && nonce.len() != HELLO_NONCE_SIZE {
        return Err(ProtocolError::InvalidNonce {
            expected: HELLO_NONCE_SIZE,
            actual: nonce.len(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod app_message_tests {
    use super::*;

    #[test]
    fn clipboard_round_trips_through_the_frame_codec() {
        let update = ClipboardUpdate {
            mime: "text/plain".to_owned(),
            payload: b"hello".to_vec(),
            sequence: 7,
        };
        let envelope = Envelope::new(
            [7_u8; REQUEST_ID_SIZE],
            Message::ClipboardUpdate(update.clone()),
        );
        envelope.validate().expect("clipboard envelope is valid");
        let frame = encode_frame(&envelope).expect("encode");
        let decoded = decode_frame(&frame).expect("decode");
        assert_eq!(decoded.message, Some(Message::ClipboardUpdate(update)));
    }

    #[test]
    fn clipboard_rejects_oversized_payload() {
        let update = ClipboardUpdate {
            mime: "text/plain".to_owned(),
            payload: vec![0_u8; MAX_FRAME_SIZE],
            sequence: 0,
        };
        let envelope = Envelope::new([0_u8; REQUEST_ID_SIZE], Message::ClipboardUpdate(update));
        assert!(matches!(
            envelope.validate(),
            Err(ProtocolError::InvalidPayload(_))
        ));
    }

    #[test]
    fn input_batch_rejects_unbounded_events_and_bad_coordinates() {
        let motion = InputEvent {
            kind: Some(input_event::Kind::MouseMove(input_event::MouseMove {
                x: 2.0,
                y: 0.5,
            })),
        };
        let batch = InputEventBatch {
            sequence: 1,
            events: vec![motion],
        };
        let envelope = Envelope::new([0_u8; REQUEST_ID_SIZE], Message::InputEventBatch(batch));
        assert!(matches!(
            envelope.validate(),
            Err(ProtocolError::InvalidPayload(_))
        ));

        let empty = InputEventBatch {
            sequence: 1,
            events: Vec::new(),
        };
        let envelope = Envelope::new([0_u8; REQUEST_ID_SIZE], Message::InputEventBatch(empty));
        assert!(matches!(
            envelope.validate(),
            Err(ProtocolError::InvalidPayload(_))
        ));
    }

    #[test]
    fn display_info_rejects_zero_dimensions() {
        let display = DisplayInfo {
            width: 0,
            height: 1080,
        };
        let envelope = Envelope::new([0_u8; REQUEST_ID_SIZE], Message::DisplayInfo(display));
        assert!(matches!(
            envelope.validate(),
            Err(ProtocolError::InvalidPayload(_))
        ));
    }

    #[test]
    fn core_conversions_round_trip() {
        let payload = deskfuse_core::ClipboardPayload::new("image/png", vec![1, 2, 3], 99)
            .expect("valid payload");
        let wire = clipboard_to_wire(&payload).expect("to wire");
        let back = clipboard_from_wire(&wire).expect("from wire");
        assert_eq!(payload, back);

        let batch = deskfuse_core::InputEventBatch::new(
            42,
            vec![
                deskfuse_core::RemoteInputEvent::MouseMove { x: 0.25, y: 0.75 },
                deskfuse_core::RemoteInputEvent::Key {
                    code: 0x24,
                    pressed: true,
                },
            ],
        )
        .expect("valid batch");
        let wire = input_batch_to_wire(&batch).expect("to wire");
        let back = input_batch_from_wire(&wire).expect("from wire");
        assert_eq!(batch, back);
    }
}

#[cfg(test)]
mod input_sharing_tests {
    use super::*;
    #[test]
    fn activation_messages_roundtrip_with_bounded_status_and_sequence() {
        use deskfuse_core::{
            Capabilities, InputActivationId, InputActivationStatus, InputShareMessage,
        };
        let activation_id = InputActivationId::new();
        for message in [
            InputShareMessage::Activate {
                activation_id,
                intent_id: InputActivationId::new(),
                ttl_ms: 1500,
                sequence: 1,
                force: false,
            },
            InputShareMessage::Activated {
                activation_id,
                ttl_ms: 1500,
                sequence: 1,
                status: InputActivationStatus::WaitingInputMonitoring,
                reason: Some("missing permission".into()),
            },
            InputShareMessage::Deactivate {
                activation_id,
                sequence: 2,
            },
        ] {
            let frame = Envelope::new(
                [4; 16],
                Message::InputSharing(InputSharing::from_core(&message).unwrap()),
            );
            let decoded = decode_frame(&encode_frame(&frame).unwrap()).unwrap();
            let Some(Message::InputSharing(wire)) = decoded.message else {
                panic!("activation lost")
            };
            assert_eq!(wire.to_core().unwrap(), message);
        }
        assert!(
            InputSharing::from_core(&InputShareMessage::Activate {
                activation_id,
                intent_id: InputActivationId::new(),
                ttl_ms: 1500,
                sequence: 0,
                force: false,
            })
            .is_err()
        );
        assert!(
            InputSharing::from_core(&InputShareMessage::Activated {
                activation_id,
                ttl_ms: 1500,
                sequence: 1,
                status: InputActivationStatus::Unavailable,
                reason: Some("x".repeat(257))
            })
            .is_err()
        );
        assert!(!Capabilities::INPUT_SHARING.contains(Capabilities::INPUT_ACTIVATION));
    }
    #[test]
    fn new_tag_roundtrips_and_tokens_are_redacted() {
        let message = deskfuse_core::InputShareMessage::Grant {
            session_id: deskfuse_core::InputSessionId::new(),
            token: deskfuse_core::InputToken([77; 32]),
            ttl_ms: 1500,
        };
        let envelope = Envelope::new(
            [1; 16],
            Message::InputSharing(InputSharing::from_core(&message).unwrap()),
        );
        let debug = format!("{envelope:?}");
        assert!(debug.contains("redacted"));
        assert!(!debug.contains("77, 77"));
        let decoded = decode_frame(&encode_frame(&envelope).unwrap()).unwrap();
        let Some(Message::InputSharing(wire)) = decoded.message else {
            panic!("new tag lost")
        };
        assert_eq!(wire.to_core().unwrap(), message);
        assert_ne!(
            deskfuse_core::Capabilities::INPUT.bits(),
            deskfuse_core::Capabilities::INPUT_SHARING.bits()
        );
    }
    #[test]
    fn malformed_and_oversized_input_fails_validation() {
        assert!(
            InputSharing {
                payload: vec![0; 65537]
            }
            .to_core()
            .is_err()
        );
        let invalid = deskfuse_core::InputShareMessage::Frame {
            session_id: deskfuse_core::InputSessionId::new(),
            token: deskfuse_core::InputToken::new(),
            target_display: "display".into(),
            sequence: 0,
            topology_generation: 0,
            events: vec![],
        };
        assert!(InputSharing::from_core(&invalid).is_err());
    }
}
