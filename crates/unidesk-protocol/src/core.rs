//! Adapters between the validated `unidesk-core` domain model and wire types.
//!
//! Core intentionally does not depend on transport or cryptography.  This
//! module is the boundary where UUID/request IDs, capability bitsets, and the
//! signed wire identity are represented in protobuf form.

use ed25519_dalek::SigningKey;
use unidesk_core::{
    Capabilities, ClipboardPayload as CoreClipboardPayload, DeviceId, DisplayInfo,
    InputEventBatch as CoreInputEventBatch, PairingRequest as CorePairingRequest, PeerSnapshot,
    ProtocolVersion, RemoteInputEvent as CoreRemoteInputEvent, RequestId,
};
use uuid::Uuid;

use crate::{
    ClipboardUpdate, Envelope, Hello, InputEvent, InputEventBatch, Message, PROTOCOL_MAJOR,
    PairingRequest, PeerInfo, ProtocolError, REQUEST_ID_SIZE, device_id_for_public_key,
    input_event, sign_hello_in_place,
};

/// Create a signed wire `Hello` from a validated core pairing request.
pub fn hello_from_core_pairing_request(
    request: &CorePairingRequest,
    nonce: [u8; crate::HELLO_NONCE_SIZE],
    signing_key: &SigningKey,
) -> Result<Hello, ProtocolError> {
    request
        .validate()
        .map_err(|error| ProtocolError::CoreConversion(error.to_string()))?;
    if u32::from(request.protocol_version().major()) != PROTOCOL_MAJOR {
        return Err(ProtocolError::IncompatibleMajor {
            received: u32::from(request.protocol_version().major()),
            supported: PROTOCOL_MAJOR,
        });
    }
    if signing_key.verifying_key().to_bytes() != *request.public_key() {
        return Err(ProtocolError::IdentityMismatch);
    }
    let mut hello = Hello {
        device_id: request.device_id().as_bytes().to_vec(),
        display_name: request.display_name().to_owned(),
        public_key: request.public_key().to_vec(),
        nonce: nonce.to_vec(),
        signature: Vec::new(),
        capabilities: request.capabilities().bits(),
    };
    sign_hello_in_place(&mut hello, signing_key)?;
    Ok(hello)
}

/// Wrap a signed core pairing request in a version-1 envelope.
pub fn envelope_from_core_pairing_request(
    request: &CorePairingRequest,
    nonce: [u8; crate::HELLO_NONCE_SIZE],
    signing_key: &SigningKey,
) -> Result<Envelope, ProtocolError> {
    let hello = hello_from_core_pairing_request(request, nonce, signing_key)?;
    Ok(Envelope {
        major: u32::from(request.protocol_version().major()),
        minor: u32::from(request.protocol_version().minor()),
        request_id: request.request_id().into_uuid().as_bytes().to_vec(),
        message: Some(Message::Hello(hello)),
    })
}

/// Convert a signed wire hello into the core pairing request model.
///
/// The envelope's version and request id are retained.  The nonce and
/// signature are transport-authentication material and are intentionally not
/// copied into the transport-independent core type.
pub fn core_pairing_request_from_envelope(
    envelope: &Envelope,
) -> Result<CorePairingRequest, ProtocolError> {
    envelope.validate()?;
    let Message::Hello(hello) = envelope
        .message
        .as_ref()
        .ok_or(ProtocolError::UnknownPayload)?
    else {
        return Err(ProtocolError::InvalidPayload(
            "envelope does not contain a hello",
        ));
    };
    core_pairing_request_from_hello(envelope, hello)
}

/// Convert a signed wire pairing-request payload into the core model.
pub fn core_pairing_request_from_pairing_envelope(
    envelope: &Envelope,
) -> Result<CorePairingRequest, ProtocolError> {
    envelope.validate()?;
    let Message::PairingRequest(request) = envelope
        .message
        .as_ref()
        .ok_or(ProtocolError::UnknownPayload)?
    else {
        return Err(ProtocolError::InvalidPayload(
            "envelope does not contain a pairing request",
        ));
    };
    let hello = Hello {
        device_id: request.device_id.clone(),
        display_name: request.display_name.clone(),
        public_key: request.public_key.clone(),
        nonce: request.nonce.clone(),
        signature: request.signature.clone(),
        capabilities: request.capabilities,
    };
    core_pairing_request_from_hello_parts(envelope, &hello)
}

fn core_pairing_request_from_hello(
    envelope: &Envelope,
    hello: &Hello,
) -> Result<CorePairingRequest, ProtocolError> {
    core_pairing_request_from_hello_parts(envelope, hello)
}

fn core_pairing_request_from_hello_parts(
    envelope: &Envelope,
    hello: &Hello,
) -> Result<CorePairingRequest, ProtocolError> {
    let public_key: [u8; 32] = hello
        .public_key
        .as_slice()
        .try_into()
        .map_err(|_| ProtocolError::InvalidPublicKey)?;
    let request_id = request_id_from_bytes(&envelope.request_id)?;
    let version = protocol_version_from_wire(envelope.major, envelope.minor)?;
    let request = CorePairingRequest::with_request_id(
        request_id,
        hello.display_name.clone(),
        public_key,
        version,
        Capabilities::from_bits(hello.capabilities),
    )
    .map_err(|error| ProtocolError::CoreConversion(error.to_string()))?;
    if request.device_id().as_bytes().as_slice() != hello.device_id.as_slice() {
        return Err(ProtocolError::IdentityMismatch);
    }
    Ok(request)
}

/// Build a public wire peer-info record from a core peer snapshot.
pub fn peer_info_from_core_snapshot(snapshot: &PeerSnapshot) -> Result<PeerInfo, ProtocolError> {
    snapshot
        .validate()
        .map_err(|error| ProtocolError::CoreConversion(error.to_string()))?;
    let capabilities = snapshot
        .routes
        .iter()
        .fold(Capabilities::NONE, |all, route| all | route.capabilities)
        .bits();
    Ok(PeerInfo {
        device_id: snapshot.id.as_bytes().to_vec(),
        display_name: snapshot.display_name.clone(),
        public_key: snapshot.public_key.to_vec(),
        capabilities,
    })
}

/// Convert a core device id to its wire representation, rechecking the
/// canonical size invariant.
#[must_use]
pub fn device_id_to_wire(device_id: DeviceId) -> Vec<u8> {
    device_id.as_bytes().to_vec()
}

/// Parse and validate a wire device id.
pub fn device_id_from_wire(bytes: &[u8]) -> Result<DeviceId, ProtocolError> {
    let raw: [u8; 32] = bytes
        .try_into()
        .map_err(|_| ProtocolError::InvalidDeviceId {
            expected: 32,
            actual: bytes.len(),
        })?;
    Ok(DeviceId::from_bytes(raw))
}

/// Convert a core request id to its exact 16-byte wire form.
#[must_use]
pub fn request_id_to_wire(request_id: RequestId) -> Vec<u8> {
    request_id.into_uuid().as_bytes().to_vec()
}

/// Parse a wire request id as a core UUID-backed request id.
pub fn request_id_from_wire(bytes: &[u8]) -> Result<RequestId, ProtocolError> {
    request_id_from_bytes(bytes)
}

fn request_id_from_bytes(bytes: &[u8]) -> Result<RequestId, ProtocolError> {
    let raw: [u8; REQUEST_ID_SIZE] =
        bytes
            .try_into()
            .map_err(|_| ProtocolError::InvalidRequestId {
                expected: REQUEST_ID_SIZE,
                actual: bytes.len(),
            })?;
    Ok(RequestId::from_uuid(Uuid::from_bytes(raw)))
}

/// Convert a core protocol version to wire numeric components.
pub fn protocol_version_to_wire(version: ProtocolVersion) -> Result<(u32, u32), ProtocolError> {
    if u32::from(version.major()) != PROTOCOL_MAJOR {
        return Err(ProtocolError::IncompatibleMajor {
            received: u32::from(version.major()),
            supported: PROTOCOL_MAJOR,
        });
    }
    Ok((u32::from(version.major()), u32::from(version.minor())))
}

/// Convert wire numeric components into a validated core version.
pub fn protocol_version_from_wire(
    major: u32,
    minor: u32,
) -> Result<ProtocolVersion, ProtocolError> {
    if major != PROTOCOL_MAJOR {
        return Err(ProtocolError::IncompatibleMajor {
            received: major,
            supported: PROTOCOL_MAJOR,
        });
    }
    let major = u16::try_from(major)
        .map_err(|_| ProtocolError::InvalidPayload("protocol major is out of range"))?;
    let minor = u16::try_from(minor)
        .map_err(|_| ProtocolError::InvalidPayload("protocol minor is out of range"))?;
    Ok(ProtocolVersion::new(major, minor))
}

/// Return the canonical device id for a core public key and verify a core id.
#[must_use]
pub fn core_device_id_matches_public_key(device_id: DeviceId, public_key: &[u8; 32]) -> bool {
    device_id.as_bytes() == &device_id_for_public_key(public_key)
}

/// Build a wire pairing request payload from a core request and already
/// authenticated hello material.
pub fn pairing_request_from_core(
    request: &CorePairingRequest,
    nonce: [u8; crate::HELLO_NONCE_SIZE],
    signature: [u8; crate::ED25519_SIGNATURE_SIZE],
) -> Result<PairingRequest, ProtocolError> {
    request
        .validate()
        .map_err(|error| ProtocolError::CoreConversion(error.to_string()))?;
    let (major, _minor) = protocol_version_to_wire(request.protocol_version())?;
    if major != PROTOCOL_MAJOR {
        return Err(ProtocolError::IncompatibleMajor {
            received: major,
            supported: PROTOCOL_MAJOR,
        });
    }
    Ok(PairingRequest {
        device_id: request.device_id().as_bytes().to_vec(),
        display_name: request.display_name().to_owned(),
        public_key: request.public_key().to_vec(),
        nonce: nonce.to_vec(),
        signature: signature.to_vec(),
        capabilities: request.capabilities().bits(),
    })
}

/// Current protocol version as a core value.
#[must_use]
pub const fn current_core_protocol_version() -> ProtocolVersion {
    ProtocolVersion::V1
}

impl TryFrom<&Envelope> for CorePairingRequest {
    type Error = ProtocolError;

    fn try_from(value: &Envelope) -> Result<Self, Self::Error> {
        core_pairing_request_from_envelope(value)
    }
}

impl TryFrom<&PeerSnapshot> for PeerInfo {
    type Error = ProtocolError;

    fn try_from(value: &PeerSnapshot) -> Result<Self, Self::Error> {
        peer_info_from_core_snapshot(value)
    }
}

impl TryFrom<Envelope> for CorePairingRequest {
    type Error = ProtocolError;

    fn try_from(value: Envelope) -> Result<Self, Self::Error> {
        core_pairing_request_from_envelope(&value)
    }
}

impl TryFrom<&CorePairingRequest> for PeerInfo {
    type Error = ProtocolError;

    fn try_from(value: &CorePairingRequest) -> Result<Self, Self::Error> {
        value
            .validate()
            .map_err(|error| ProtocolError::CoreConversion(error.to_string()))?;
        Ok(PeerInfo {
            device_id: value.device_id().as_bytes().to_vec(),
            display_name: value.display_name().to_owned(),
            public_key: value.public_key().to_vec(),
            capabilities: value.capabilities().bits(),
        })
    }
}

/// Convert a validated core clipboard payload into its wire representation.
pub fn clipboard_to_wire(payload: &CoreClipboardPayload) -> Result<ClipboardUpdate, ProtocolError> {
    payload
        .validate()
        .map_err(|error| ProtocolError::CoreConversion(error.to_string()))?;
    Ok(ClipboardUpdate {
        mime: payload.mime.clone(),
        payload: payload.data.clone(),
        sequence: payload.sequence,
    })
}

/// Convert a wire clipboard update into a validated core payload.
pub fn clipboard_from_wire(
    update: &ClipboardUpdate,
) -> Result<CoreClipboardPayload, ProtocolError> {
    CoreClipboardPayload::new(update.mime.clone(), update.payload.clone(), update.sequence)
        .map_err(|error| ProtocolError::CoreConversion(error.to_string()))
}

/// Convert a validated core input batch into its wire representation.
pub fn input_batch_to_wire(batch: &CoreInputEventBatch) -> Result<InputEventBatch, ProtocolError> {
    batch
        .validate()
        .map_err(|error| ProtocolError::CoreConversion(error.to_string()))?;
    let mut events = Vec::with_capacity(batch.events.len());
    for event in &batch.events {
        let kind = match event {
            CoreRemoteInputEvent::MouseMove { x, y } => {
                input_event::Kind::MouseMove(input_event::MouseMove { x: *x, y: *y })
            }
            CoreRemoteInputEvent::MouseButton { button, pressed } => {
                input_event::Kind::MouseButton(input_event::MouseButton {
                    button: u32::from(*button),
                    pressed: *pressed,
                })
            }
            CoreRemoteInputEvent::Key { code, pressed } => {
                input_event::Kind::Key(input_event::Key {
                    code: u32::from(*code),
                    pressed: *pressed,
                })
            }
            CoreRemoteInputEvent::Scroll { delta_x, delta_y } => {
                input_event::Kind::Scroll(input_event::Scroll {
                    delta_x: *delta_x,
                    delta_y: *delta_y,
                })
            }
        };
        events.push(InputEvent { kind: Some(kind) });
    }
    Ok(InputEventBatch {
        sequence: batch.sequence,
        events,
    })
}

/// Convert a wire input batch into validated core events.
pub fn input_batch_from_wire(
    batch: &InputEventBatch,
) -> Result<CoreInputEventBatch, ProtocolError> {
    let mut events = Vec::with_capacity(batch.events.len());
    for event in &batch.events {
        let kind = event.kind.as_ref().ok_or(ProtocolError::UnknownPayload)?;
        let core_event = match kind {
            input_event::Kind::MouseMove(motion) => CoreRemoteInputEvent::MouseMove {
                x: motion.x,
                y: motion.y,
            },
            input_event::Kind::MouseButton(button) => {
                let button_number = u8::try_from(button.button).map_err(|_| {
                    ProtocolError::InvalidPayload("mouse button number is too large")
                })?;
                CoreRemoteInputEvent::MouseButton {
                    button: button_number,
                    pressed: button.pressed,
                }
            }
            input_event::Kind::Key(key) => {
                let code = u16::try_from(key.code)
                    .map_err(|_| ProtocolError::InvalidPayload("key code is too large"))?;
                CoreRemoteInputEvent::Key {
                    code,
                    pressed: key.pressed,
                }
            }
            input_event::Kind::Scroll(scroll) => CoreRemoteInputEvent::Scroll {
                delta_x: scroll.delta_x,
                delta_y: scroll.delta_y,
            },
        };
        core_event
            .validate()
            .map_err(|error| ProtocolError::CoreConversion(error.to_string()))?;
        events.push(core_event);
    }
    CoreInputEventBatch::new(batch.sequence, events)
        .map_err(|error| ProtocolError::CoreConversion(error.to_string()))
}

/// Convert a validated core display snapshot into its wire representation.
pub fn display_info_to_wire(
    display: DisplayInfo,
) -> Result<crate::wire::DisplayInfo, ProtocolError> {
    if display.width == 0 || display.height == 0 {
        return Err(ProtocolError::CoreConversion(
            "display dimensions must be non-zero".to_owned(),
        ));
    }
    Ok(crate::wire::DisplayInfo {
        width: display.width,
        height: display.height,
    })
}

/// Convert a wire display snapshot into a validated core value.
pub fn display_info_from_wire(
    display: &crate::wire::DisplayInfo,
) -> Result<DisplayInfo, ProtocolError> {
    DisplayInfo::new(display.width, display.height)
        .map_err(|error| ProtocolError::CoreConversion(error.to_string()))
}
