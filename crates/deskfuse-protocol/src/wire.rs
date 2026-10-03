//! Protobuf wire messages for the DeskFuse peer protocol.
//!
//! The wire types in this module are deliberately hand-written.  This keeps
//! the protocol crate usable in environments where `prost-build` and a
//! `protoc` binary are not available while still using prost's canonical
//! protobuf encoder/decoder at runtime.

use crate::update::{
    ReleaseInventory, UpdateCancel, UpdateChunk, UpdateReceipt, UpdateRequest, UpdateStatus,
};
use prost::Message;

/// The top-level protocol envelope.
#[derive(Clone, PartialEq, Message)]
pub struct Envelope {
    /// The protocol major version negotiated by the sender.
    #[prost(uint32, tag = "1")]
    pub major: u32,
    /// The protocol minor version negotiated by the sender.
    #[prost(uint32, tag = "2")]
    pub minor: u32,
    /// An opaque, protocol-level request identifier.  The protocol currently
    /// uses 16-byte identifiers (UUID-sized, but not tied to UUID encoding).
    #[prost(bytes = "vec", tag = "3")]
    pub request_id: Vec<u8>,
    /// The message carried by this envelope.
    #[prost(
        oneof = "envelope::Message",
        tags = "10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 21, 22, 30, 31, 32, 33, 34, 40, 41, 42, 43, 44, 45, 46"
    )]
    pub message: Option<envelope::Message>,
}

pub mod envelope {
    use super::{
        ClipboardUpdate, DisplayInfo, Error, Goodbye, Hello, HelloAck, InputEndpoints,
        InputEventBatch, InputSharing, NetworkControl, PairingDecision, PairingRequest, PeerInfo,
        Ping, Pong, ReleaseInventory, RemoteApplicationOpen, SessionProof, UpdateCancel,
        UpdateChunk, UpdateReceipt, UpdateRequest, UpdateStatus, WorkflowSync,
    };
    use prost::Oneof;

    /// The closed set of messages understood by protocol version 2.
    #[derive(Clone, PartialEq, Oneof)]
    pub enum Message {
        #[prost(message, tag = "10")]
        Hello(Hello),
        #[prost(message, tag = "11")]
        HelloAck(HelloAck),
        #[prost(message, tag = "12")]
        Ping(Ping),
        #[prost(message, tag = "13")]
        Pong(Pong),
        #[prost(message, tag = "14")]
        PeerInfo(PeerInfo),
        #[prost(message, tag = "15")]
        PairingRequest(PairingRequest),
        #[prost(message, tag = "16")]
        PairingDecision(PairingDecision),
        #[prost(message, tag = "17")]
        Error(Error),
        #[prost(message, tag = "18")]
        Goodbye(Goodbye),
        #[prost(message, tag = "20")]
        ClipboardUpdate(ClipboardUpdate),
        #[prost(message, tag = "21")]
        InputEventBatch(InputEventBatch),
        #[prost(message, tag = "22")]
        DisplayInfo(DisplayInfo),
        #[prost(message, tag = "30")]
        InputSharing(InputSharing),
        #[prost(message, tag = "31")]
        InputEndpoints(InputEndpoints),
        #[prost(message, tag = "32")]
        NetworkControl(NetworkControl),
        #[prost(message, tag = "33")]
        SessionProof(SessionProof),
        #[prost(message, tag = "34")]
        RemoteApplicationOpen(RemoteApplicationOpen),
        #[prost(message, tag = "40")]
        ReleaseInventory(ReleaseInventory),
        #[prost(message, tag = "41")]
        UpdateRequest(UpdateRequest),
        #[prost(message, tag = "42")]
        UpdateChunk(UpdateChunk),
        #[prost(message, tag = "43")]
        UpdateCancel(UpdateCancel),
        #[prost(message, tag = "44")]
        UpdateStatus(UpdateStatus),
        #[prost(message, tag = "45")]
        UpdateReceipt(UpdateReceipt),
        /// Bounded, authenticated workflow-catalog anti-entropy frame.
        #[prost(message, tag = "46")]
        WorkflowSync(WorkflowSync),
    }
}

/// The initial signed identity message exchanged by peers.
#[derive(Clone, PartialEq, Message)]
pub struct Hello {
    /// SHA-256(public_key), encoded as 32 raw bytes.
    #[prost(bytes = "vec", tag = "1")]
    pub device_id: Vec<u8>,
    #[prost(string, tag = "2")]
    pub display_name: String,
    /// An Ed25519 public key, encoded in its 32-byte compressed form.
    #[prost(bytes = "vec", tag = "3")]
    pub public_key: Vec<u8>,
    /// A fresh challenge nonce.  The implementation requires 32 bytes.
    #[prost(bytes = "vec", tag = "4")]
    pub nonce: Vec<u8>,
    /// An Ed25519 signature over [`canonical_hello_payload`].
    #[prost(bytes = "vec", tag = "5")]
    pub signature: Vec<u8>,
    #[prost(uint64, tag = "6")]
    pub capabilities: u64,
}

/// A response to [`Hello`].
#[derive(Clone, PartialEq, Message)]
pub struct HelloAck {
    #[prost(bytes = "vec", tag = "1")]
    pub device_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    pub nonce: Vec<u8>,
    #[prost(bytes = "vec", tag = "3")]
    pub signature: Vec<u8>,
    #[prost(bool, tag = "4")]
    pub accepted: bool,
    #[prost(uint64, tag = "5")]
    pub capabilities: u64,
}

/// Liveness challenge.
#[derive(Clone, PartialEq, Message)]
pub struct Ping {
    /// Optional opaque challenge.  A missing challenge is valid for a simple
    /// ping, while a present challenge is echoed by [`Pong`].
    #[prost(bytes = "vec", tag = "1")]
    pub nonce: Vec<u8>,
}

/// Liveness response.
#[derive(Clone, PartialEq, Message)]
pub struct Pong {
    #[prost(bytes = "vec", tag = "1")]
    pub nonce: Vec<u8>,
}

/// Public information about a known peer.
#[derive(Clone, PartialEq, Message)]
pub struct PeerInfo {
    #[prost(bytes = "vec", tag = "1")]
    pub device_id: Vec<u8>,
    #[prost(string, tag = "2")]
    pub display_name: String,
    #[prost(bytes = "vec", tag = "3")]
    pub public_key: Vec<u8>,
    #[prost(uint64, tag = "4")]
    pub capabilities: u64,
}

/// A request to pair with the sender.
#[derive(Clone, PartialEq, Message)]
pub struct PairingRequest {
    #[prost(bytes = "vec", tag = "1")]
    pub device_id: Vec<u8>,
    #[prost(string, tag = "2")]
    pub display_name: String,
    #[prost(bytes = "vec", tag = "3")]
    pub public_key: Vec<u8>,
    #[prost(bytes = "vec", tag = "4")]
    pub nonce: Vec<u8>,
    #[prost(bytes = "vec", tag = "5")]
    pub signature: Vec<u8>,
    #[prost(uint64, tag = "6")]
    pub capabilities: u64,
}

/// The local user's decision about a pairing request.
#[derive(Clone, PartialEq, Message)]
pub struct PairingDecision {
    #[prost(bytes = "vec", tag = "1")]
    pub device_id: Vec<u8>,
    #[prost(bool, tag = "2")]
    pub accepted: bool,
    #[prost(string, tag = "3")]
    pub reason: String,
    #[prost(bytes = "vec", tag = "4")]
    pub nonce: Vec<u8>,
    #[prost(bytes = "vec", tag = "5")]
    pub signature: Vec<u8>,
}

/// A typed protocol/application error.
#[derive(Clone, PartialEq, Message)]
pub struct Error {
    #[prost(uint32, tag = "1")]
    pub code: u32,
    #[prost(string, tag = "2")]
    pub message: String,
    #[prost(bool, tag = "3")]
    pub retryable: bool,
}

/// Graceful session termination.
#[derive(Clone, PartialEq, Message)]
pub struct Goodbye {
    #[prost(string, tag = "1")]
    pub reason: String,
}

/// A clipboard payload pushed from one paired device to another.
#[derive(Clone, PartialEq, Message)]
pub struct ClipboardUpdate {
    /// MIME type, for example `text/plain` or `image/png`.
    #[prost(string, tag = "1")]
    pub mime: String,
    /// Clipboard body bytes. Diagnostics and logs must never retain this.
    #[prost(bytes = "vec", tag = "2")]
    pub payload: Vec<u8>,
    /// Monotonic sender-local sequence number used for echo suppression.
    #[prost(uint64, tag = "3")]
    pub sequence: u64,
}

/// A bounded batch of normalized input events for the remote-control boundary.
#[derive(Clone, PartialEq, Message)]
pub struct InputEventBatch {
    #[prost(uint64, tag = "1")]
    pub sequence: u64,
    #[prost(message, repeated, tag = "2")]
    pub events: Vec<InputEvent>,
}

/// One normalized remote input event.
#[derive(Clone, PartialEq, Message)]
pub struct InputEvent {
    #[prost(oneof = "input_event::Kind", tags = "1, 2, 3, 4")]
    pub kind: Option<input_event::Kind>,
}

/// Nested oneof module required by prost.
pub mod input_event {
    use prost::{Message, Oneof};

    /// Cursor position normalized to the sender's main display.
    #[derive(Clone, PartialEq, Message)]
    pub struct MouseMove {
        /// Normalized x in `0.0..=1.0`.
        #[prost(double, tag = "1")]
        pub x: f64,
        /// Normalized y in `0.0..=1.0`.
        #[prost(double, tag = "2")]
        pub y: f64,
    }

    /// Mouse button state change.
    #[derive(Clone, PartialEq, Message)]
    pub struct MouseButton {
        /// Button number: 0 left, 1 right, 2 middle, higher values vendor-specific.
        #[prost(uint32, tag = "1")]
        pub button: u32,
        #[prost(bool, tag = "2")]
        pub pressed: bool,
    }

    /// Keyboard key state change.
    #[derive(Clone, PartialEq, Message)]
    pub struct Key {
        /// macOS virtual key code.
        #[prost(uint32, tag = "1")]
        pub code: u32,
        #[prost(bool, tag = "2")]
        pub pressed: bool,
    }

    /// Scroll-wheel deltas in lines.
    #[derive(Clone, PartialEq, Message)]
    pub struct Scroll {
        #[prost(double, tag = "1")]
        pub delta_x: f64,
        #[prost(double, tag = "2")]
        pub delta_y: f64,
    }

    #[derive(Clone, PartialEq, Oneof)]
    pub enum Kind {
        #[prost(message, tag = "1")]
        MouseMove(MouseMove),
        #[prost(message, tag = "2")]
        MouseButton(MouseButton),
        #[prost(message, tag = "3")]
        Key(Key),
        #[prost(message, tag = "4")]
        Scroll(Scroll),
    }
}

/// Main-display geometry used to map a controller cursor onto the controlled
/// device. Coordinates are physical pixels.
#[derive(Clone, PartialEq, Message)]
pub struct DisplayInfo {
    #[prost(uint32, tag = "1")]
    pub width: u32,
    #[prost(uint32, tag = "2")]
    pub height: u32,
}

/// Bounded workflow-catalog anti-entropy data. The payload is the catalog's
/// authenticated record frame; it is never interpreted by the wire layer.
#[derive(Clone, PartialEq, Message)]
#[prost(skip_debug)]
pub struct WorkflowSync {
    #[prost(bytes = "vec", tag = "1")]
    pub payload: Vec<u8>,
}
impl std::fmt::Debug for WorkflowSync {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WorkflowSync([redacted])")
    }
}

/// Capability-gated version-one input sharing payload. Legacy tags remain unchanged.
#[derive(Clone, PartialEq, Message)]
#[prost(skip_debug)]
pub struct InputSharing {
    #[prost(bytes = "vec", tag = "1")]
    pub payload: Vec<u8>,
}
impl std::fmt::Debug for InputSharing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InputSharing([redacted])")
    }
}
/// A 64 KiB JPEG is base64-encoded inside JSON, so its wire payload can
/// exceed the ordinary 64 KiB input bound without exceeding 96 KiB.
pub const MAX_INPUT_WALLPAPER_PAYLOAD: usize = 96 * 1024;
const MAX_ORDINARY_INPUT_PAYLOAD: usize = 64 * 1024;

impl InputSharing {
    pub fn from_core(
        message: &deskfuse_core::InputShareMessage,
    ) -> Result<Self, crate::ProtocolError> {
        message
            .validate()
            .map_err(|_| crate::ProtocolError::InvalidPayload("invalid input sharing message"))?;
        let payload = serde_json::to_vec(message)
            .map_err(|_| crate::ProtocolError::InvalidPayload("input serialization failed"))?;
        let limit = if matches!(
            message,
            deskfuse_core::InputShareMessage::DisplayWallpaper { .. }
        ) {
            MAX_INPUT_WALLPAPER_PAYLOAD
        } else {
            MAX_ORDINARY_INPUT_PAYLOAD
        };
        if payload.len() > limit {
            return Err(crate::ProtocolError::InvalidPayload(
                "input payload too large",
            ));
        }
        Ok(Self { payload })
    }
    pub fn to_core(&self) -> Result<deskfuse_core::InputShareMessage, crate::ProtocolError> {
        if self.payload.len() > MAX_INPUT_WALLPAPER_PAYLOAD {
            return Err(crate::ProtocolError::InvalidPayload(
                "input payload too large",
            ));
        }
        let message: deskfuse_core::InputShareMessage = serde_json::from_slice(&self.payload)
            .map_err(|_| crate::ProtocolError::InvalidPayload("invalid input payload"))?;
        if self.payload.len() > MAX_ORDINARY_INPUT_PAYLOAD
            && !matches!(
                message,
                deskfuse_core::InputShareMessage::DisplayWallpaper { .. }
            )
        {
            return Err(crate::ProtocolError::InvalidPayload(
                "input payload too large",
            ));
        }
        message
            .validate()
            .map_err(|_| crate::ProtocolError::InvalidPayload("invalid input sharing message"))?;
        Ok(message)
    }
}

/// Authenticated transport RPC. An empty list requests peer LAN listeners.
#[derive(Clone, PartialEq, Message)]
pub struct InputEndpoints {
    #[prost(string, repeated, tag = "1")]
    pub addresses: Vec<String>,
}

/// Signed device-network management, carried only by the isolated pairing channel.
#[derive(Clone, PartialEq, Message)]
pub struct NetworkControl {
    #[prost(bytes = "vec", tag = "1")]
    pub payload: Vec<u8>,
}

/// Fresh Ed25519 proof bound to the exact QUIC exporter and Hello transcript.
#[derive(Clone, PartialEq, Message)]
pub struct SessionProof {
    #[prost(bytes = "vec", tag = "1")]
    pub signature: Vec<u8>,
}

#[cfg(test)]
mod motion_control_tests {
    use super::InputSharing;
    use deskfuse_core::{
        DisplayId, HybridInputUpdate, InputSessionId, InputShareMessage, InputToken,
        MotionPosition, SharedInputEvent,
    };

    #[test]
    fn reliable_hybrid_controls_roundtrip_the_existing_envelope() {
        let id = InputSessionId::new();
        let token = InputToken::new();
        let position = MotionPosition {
            target_display: DisplayId::new("main"),
            topology_generation: 1,
            motion_seq: 4,
            x: 10.,
            y: 20.,
        };
        let messages = [
            InputShareMessage::RequestHybrid {
                session_id: id,
                target_display: DisplayId::new("main"),
            },
            InputShareMessage::GrantHybrid {
                session_id: id,
                token,
                ttl_ms: 1500,
                motion_enabled: true,
            },
            InputShareMessage::Hybrid {
                session_id: id,
                token,
                sequence: 5,
                update: HybridInputUpdate::Enter {
                    position: position.clone(),
                },
            },
            InputShareMessage::Hybrid {
                session_id: id,
                token,
                sequence: 6,
                update: HybridInputUpdate::Transition {
                    position: position.clone(),
                    events: vec![SharedInputEvent::Button {
                        button: 0,
                        pressed: true,
                    }],
                },
            },
            InputShareMessage::Hybrid {
                session_id: id,
                token,
                sequence: 8,
                update: HybridInputUpdate::Snapshot {
                    state_epoch: 6,
                    position,
                },
            },
        ];
        for message in messages {
            let wire = InputSharing::from_core(&message).unwrap();
            assert_eq!(format!("{wire:?}"), "InputSharing([redacted])");
            assert_eq!(wire.to_core().unwrap(), message);
        }
    }

    #[test]
    fn old_request_and_grant_remain_unchanged() {
        let id = InputSessionId::new();
        for message in [
            InputShareMessage::Request {
                session_id: id,
                target_display: DisplayId::new("main"),
            },
            InputShareMessage::Grant {
                session_id: id,
                token: InputToken::new(),
                ttl_ms: 1500,
            },
        ] {
            assert_eq!(
                InputSharing::from_core(&message)
                    .unwrap()
                    .to_core()
                    .unwrap(),
                message
            );
        }
    }

    #[test]
    fn transition_motion_is_rejected_on_both_encoding_and_untrusted_decode() {
        let position = MotionPosition {
            target_display: DisplayId::new("main"),
            topology_generation: 1,
            motion_seq: 4,
            x: 10.,
            y: 20.,
        };
        let boundary = SharedInputEvent::Button {
            button: 0,
            pressed: true,
        };
        for events in [
            vec![SharedInputEvent::Motion { x: 30., y: 40. }],
            vec![
                SharedInputEvent::Motion { x: 30., y: 40. },
                boundary.clone(),
            ],
            vec![
                boundary.clone(),
                SharedInputEvent::Motion { x: 30., y: 40. },
                boundary.clone(),
            ],
            vec![boundary, SharedInputEvent::Motion { x: 30., y: 40. }],
        ] {
            let message = InputShareMessage::Hybrid {
                session_id: InputSessionId::new(),
                token: InputToken::new(),
                sequence: 5,
                update: HybridInputUpdate::Transition {
                    position: position.clone(),
                    events,
                },
            };
            assert!(InputSharing::from_core(&message).is_err());
            // Bypass the safe encoder to exercise actual remote-payload validation.
            let untrusted = InputSharing {
                payload: serde_json::to_vec(&message).unwrap(),
            };
            assert!(untrusted.to_core().is_err());
        }
    }
}

/// Upgrade one authenticated control stream into a bounded remote media session.
#[derive(Clone, PartialEq, Message)]
pub struct RemoteApplicationOpen {
    #[prost(uint32, tag = "1")]
    pub version: u32,
}
