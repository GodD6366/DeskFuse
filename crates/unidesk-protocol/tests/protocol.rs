use ed25519_dalek::SigningKey;
use prost::Message as ProstMessage;
use tokio_util::codec::Decoder;
use uuid::Uuid;

use unidesk_core::{
    Capabilities, PairingRequest as CorePairingRequest, ProtocolVersion, RequestId,
};
use unidesk_protocol::{
    Envelope, FrameCodec, Hello, MAX_FRAME_SIZE, Message, PROTOCOL_MAJOR, PROTOCOL_MINOR, Ping,
    ProtocolError, REQUEST_ID_SIZE, decode_frame, device_id_for_public_key, encode_frame,
    envelope_from_core_pairing_request, hello_from_core_pairing_request, sign_hello,
    sign_hello_in_place, verify_hello,
};

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7_u8; 32])
}

fn signed_hello() -> (Hello, SigningKey) {
    let key = signing_key();
    let public_key = key.verifying_key().to_bytes();
    let mut hello = Hello {
        device_id: device_id_for_public_key(&public_key).to_vec(),
        display_name: "Test Mac".to_owned(),
        public_key: public_key.to_vec(),
        nonce: vec![0xAB; 32],
        signature: Vec::new(),
        capabilities: 0b101,
    };
    sign_hello_in_place(&mut hello, &key).expect("hello must sign");
    (hello, key)
}

fn envelope(message: Message) -> Envelope {
    Envelope::new([0x11; REQUEST_ID_SIZE], message)
}

#[test]
fn signed_hello_roundtrips_and_verifies() {
    let (hello, _) = signed_hello();
    verify_hello(&hello).expect("signature must verify");

    let frame = encode_frame(&envelope(Message::Hello(hello.clone()))).expect("encode");
    let decoded = decode_frame(&frame).expect("decode");
    assert_eq!(decoded.message, Some(Message::Hello(hello)));
}

#[test]
fn sign_hello_returns_detached_signature() {
    let (mut hello, key) = signed_hello();
    hello.signature.clear();
    let signature = sign_hello(&hello, &key).expect("detached signature");
    assert_eq!(signature.len(), 64);
    hello.signature = signature;
    verify_hello(&hello).expect("detached signature must verify");
}

#[test]
fn hello_tampering_is_rejected() {
    let (mut hello, _) = signed_hello();
    hello.display_name.push('!');
    assert!(matches!(
        verify_hello(&hello),
        Err(ProtocolError::SignatureVerification)
    ));

    let frame = encode_frame(&envelope(Message::Hello(hello)));
    assert!(matches!(frame, Err(ProtocolError::SignatureVerification)));
}

#[test]
fn hello_identity_mismatch_is_rejected_before_signature() {
    let (mut hello, _) = signed_hello();
    hello.device_id[0] ^= 0xFF;
    assert!(matches!(
        verify_hello(&hello),
        Err(ProtocolError::IdentityMismatch)
    ));
}

#[test]
fn frame_codec_uses_big_endian_u32_length_prefix() {
    let frame = encode_frame(&envelope(Message::Ping(Ping { nonce: vec![] }))).expect("encode");
    assert_eq!(
        u32::from_be_bytes(frame[..4].try_into().unwrap()) as usize,
        frame.len() - 4
    );
    assert_eq!(frame[0], 0);

    let mut codec = FrameCodec::new();
    let mut input = bytes::BytesMut::from(&frame[..]);
    assert_eq!(
        codec.decode(&mut input).expect("decode"),
        Some(envelope(Message::Ping(Ping { nonce: vec![] })))
    );
    assert!(input.is_empty());
}

#[test]
fn truncated_frames_are_rejected() {
    assert!(matches!(
        decode_frame(&[0, 0, 0]),
        Err(ProtocolError::Truncated { .. })
    ));

    let frame = encode_frame(&envelope(Message::Ping(Ping { nonce: vec![] }))).expect("encode");
    assert!(matches!(
        decode_frame(&frame[..frame.len() - 1]),
        Err(ProtocolError::Truncated { .. })
    ));
}

#[test]
fn oversized_frames_are_rejected_before_payload_decode() {
    let oversized =
        (u32::try_from(MAX_FRAME_SIZE).expect("max frame fits in u32") + 1).to_be_bytes();
    assert!(matches!(
        decode_frame(&oversized),
        Err(ProtocolError::FrameTooLarge { .. })
    ));

    let mut codec = FrameCodec::new();
    let mut input = bytes::BytesMut::from(&oversized[..]);
    assert!(matches!(
        codec.decode(&mut input),
        Err(ProtocolError::FrameTooLarge { .. })
    ));
}

#[test]
fn incompatible_major_versions_are_rejected() {
    let mut payload = Vec::new();
    Envelope {
        major: PROTOCOL_MAJOR + 1,
        minor: PROTOCOL_MINOR,
        request_id: vec![0x22; REQUEST_ID_SIZE],
        message: Some(Message::Ping(Ping { nonce: Vec::new() })),
    }
    .encode(&mut payload)
    .expect("encode");
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("test payload fits in u32")
            .to_be_bytes(),
    );
    frame.extend_from_slice(&payload);

    assert!(matches!(
        decode_frame(&frame),
        Err(ProtocolError::IncompatibleMajor { .. })
    ));
}

#[test]
fn unknown_oneof_payload_is_rejected() {
    let mut payload = Vec::new();
    Envelope {
        major: PROTOCOL_MAJOR,
        minor: PROTOCOL_MINOR,
        request_id: vec![0x22; REQUEST_ID_SIZE],
        message: None,
    }
    .encode(&mut payload)
    .expect("encode");
    // Field 99, length-delimited: it is unknown to this protocol version.
    payload.extend_from_slice(&[0x9A, 0x06, 0x01, 0xFF]);

    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("test payload fits in u32")
            .to_be_bytes(),
    );
    frame.extend_from_slice(&payload);
    assert!(matches!(
        decode_frame(&frame),
        Err(ProtocolError::UnknownPayload)
    ));
}

#[test]
fn illegal_request_ids_are_rejected() {
    let invalid = Envelope {
        major: PROTOCOL_MAJOR,
        minor: PROTOCOL_MINOR,
        request_id: vec![1; REQUEST_ID_SIZE - 1],
        message: Some(Message::Ping(Ping { nonce: Vec::new() })),
    };
    assert!(matches!(
        encode_frame(&invalid),
        Err(ProtocolError::InvalidRequestId { .. })
    ));
}

#[test]
fn core_pairing_request_conversion_preserves_domain_values() {
    let key = signing_key();
    let public_key = key.verifying_key().to_bytes();
    let request_id = RequestId::from_uuid(Uuid::from_bytes([0x44; 16]));
    let request = CorePairingRequest::with_request_id(
        request_id,
        "Test Mac",
        public_key,
        ProtocolVersion::V1,
        Capabilities::NETWORK | Capabilities::TRANSFER,
    )
    .expect("valid core request");

    let hello = hello_from_core_pairing_request(&request, [0xCD; 32], &key).expect("hello");
    verify_hello(&hello).expect("converted hello must verify");
    let envelope =
        envelope_from_core_pairing_request(&request, [0xCD; 32], &key).expect("envelope");
    let decoded =
        unidesk_protocol::core_pairing_request_from_envelope(&envelope).expect("core conversion");

    assert_eq!(decoded.request_id(), request.request_id());
    assert_eq!(decoded.device_id(), request.device_id());
    assert_eq!(decoded.display_name(), request.display_name());
    assert_eq!(decoded.public_key(), request.public_key());
    assert_eq!(decoded.protocol_version(), request.protocol_version());
    assert_eq!(decoded.capabilities(), request.capabilities());
}
