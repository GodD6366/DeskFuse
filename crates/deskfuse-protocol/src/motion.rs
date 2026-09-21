//! Small, independent binary framing for mouse-position QUIC datagrams.
//!
//! These packets never enter the reliable RPC frame codec or JSON decoder. The
//! prefix identifies kind/version; an exact bounded payload length is checked
//! before prost may allocate. Authentication is the enclosing QUIC peer plus
//! the current session token, checked by the input runtime after decoding.

use deskfuse_core::{DisplayId, InputSessionId, InputToken, MotionDatagram, MotionPosition};
use prost::Message;

use crate::ProtocolError;

/// Maximum complete motion packet size, including kind/version and length.
pub const MAX_MOTION_DATAGRAM_SIZE: usize = 512;
// DF (DeskFuse), M (motion), framing version 1.
const PREFIX: [u8; 4] = [b'D', b'F', b'M', 1];
const HEADER_SIZE: usize = 6;

#[derive(Clone, PartialEq, Message)]
struct MotionPacket {
    #[prost(bytes = "vec", tag = "1")]
    session_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    token: Vec<u8>,
    #[prost(uint64, tag = "3")]
    state_epoch: u64,
    #[prost(string, tag = "4")]
    target_display: String,
    #[prost(uint64, tag = "5")]
    topology_generation: u64,
    #[prost(uint64, tag = "6")]
    motion_seq: u64,
    #[prost(double, tag = "7")]
    x: f64,
    #[prost(double, tag = "8")]
    y: f64,
}

/// Encode a validated bounded latest-position datagram.
pub fn encode_motion_datagram(motion: &MotionDatagram) -> Result<Vec<u8>, ProtocolError> {
    motion
        .validate()
        .map_err(|_| ProtocolError::InvalidPayload("invalid motion datagram"))?;
    let packet = MotionPacket {
        session_id: motion.session_id.0.as_bytes().to_vec(),
        token: motion.token.0.to_vec(),
        state_epoch: motion.state_epoch,
        target_display: motion.position.target_display.0.clone(),
        topology_generation: motion.position.topology_generation,
        motion_seq: motion.position.motion_seq,
        x: motion.position.x,
        y: motion.position.y,
    };
    let payload_size = packet.encoded_len();
    check_size(HEADER_SIZE + payload_size)?;
    let mut bytes = Vec::with_capacity(HEADER_SIZE + payload_size);
    bytes.extend_from_slice(&PREFIX);
    bytes.extend_from_slice(&(payload_size as u16).to_be_bytes());
    packet.encode(&mut bytes)?;
    Ok(bytes)
}

/// Decode one whole datagram; rejects truncation, concatenation and other kinds.
pub fn decode_motion_datagram(bytes: &[u8]) -> Result<MotionDatagram, ProtocolError> {
    check_size(bytes.len())?;
    if bytes.len() < HEADER_SIZE {
        return Err(ProtocolError::Truncated {
            expected: HEADER_SIZE,
            actual: bytes.len(),
        });
    }
    if bytes[..4] != PREFIX {
        return Err(ProtocolError::InvalidPayload(
            "unknown motion kind or version",
        ));
    }
    let declared = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
    let expected = HEADER_SIZE + declared;
    check_size(expected)?;
    if bytes.len() < expected {
        return Err(ProtocolError::Truncated {
            expected,
            actual: bytes.len(),
        });
    }
    if bytes.len() > expected {
        return Err(ProtocolError::TrailingBytes {
            remaining: bytes.len() - expected,
        });
    }
    let packet = MotionPacket::decode(&bytes[HEADER_SIZE..])?;
    let session_id = uuid::Uuid::from_slice(&packet.session_id)
        .map_err(|_| ProtocolError::InvalidPayload("invalid motion session ID"))?;
    let token: [u8; 32] = packet
        .token
        .as_slice()
        .try_into()
        .map_err(|_| ProtocolError::InvalidPayload("invalid motion token length"))?;
    let motion = MotionDatagram {
        session_id: InputSessionId(session_id),
        token: InputToken(token),
        state_epoch: packet.state_epoch,
        position: MotionPosition {
            target_display: DisplayId(packet.target_display),
            topology_generation: packet.topology_generation,
            motion_seq: packet.motion_seq,
            x: packet.x,
            y: packet.y,
        },
    };
    motion
        .validate()
        .map_err(|_| ProtocolError::InvalidPayload("invalid motion datagram"))?;
    Ok(motion)
}

fn check_size(size: usize) -> Result<(), ProtocolError> {
    if size > MAX_MOTION_DATAGRAM_SIZE {
        Err(ProtocolError::FrameTooLarge {
            size,
            max: MAX_MOTION_DATAGRAM_SIZE,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MotionDatagram {
        MotionDatagram {
            session_id: InputSessionId::new(),
            token: InputToken::new(),
            state_epoch: 2,
            position: MotionPosition {
                target_display: DisplayId("display-1".into()),
                topology_generation: 7,
                motion_seq: 11,
                x: 12.5,
                y: 24.25,
            },
        }
    }

    #[test]
    fn binary_packet_roundtrips_and_stays_small() {
        let value = sample();
        let encoded = encode_motion_datagram(&value).unwrap();
        assert_eq!(encoded[..4], PREFIX);
        assert!(encoded.len() < 128);
        assert_eq!(decode_motion_datagram(&encoded).unwrap(), value);
    }

    #[test]
    fn worst_case_valid_packet_fits_the_negotiated_bound() {
        let mut value = sample();
        value.state_epoch = u64::MAX;
        value.position.topology_generation = u64::MAX;
        value.position.motion_seq = u64::MAX;
        value.position.target_display = DisplayId("x".repeat(128));
        value.position.x = 65536.;
        let bytes = encode_motion_datagram(&value).unwrap();
        assert!(bytes.len() <= MAX_MOTION_DATAGRAM_SIZE);
        assert_eq!(decode_motion_datagram(&bytes).unwrap(), value);
    }

    #[test]
    fn rejects_every_truncation_and_trailing_bytes() {
        let bytes = encode_motion_datagram(&sample()).unwrap();
        for len in 0..bytes.len() {
            assert!(decode_motion_datagram(&bytes[..len]).is_err());
        }
        let mut extra = bytes;
        extra.push(0);
        assert!(matches!(
            decode_motion_datagram(&extra),
            Err(ProtocolError::TrailingBytes { .. })
        ));
    }

    #[test]
    fn rejects_invalid_values_on_encode_and_decode() {
        let mut value = sample();
        value.position.x = f64::NAN;
        assert!(encode_motion_datagram(&value).is_err());
        value.position.x = 0.;
        value.state_epoch = 0;
        assert!(encode_motion_datagram(&value).is_err());
        let bytes = encode_motion_datagram(&sample()).unwrap();
        let mut packet = MotionPacket::decode(&bytes[HEADER_SIZE..]).unwrap();
        for token_len in [0, 31, 33] {
            packet.token = vec![1; token_len];
            let payload = packet.encode_to_vec();
            let mut invalid = PREFIX.to_vec();
            invalid.extend_from_slice(&(payload.len() as u16).to_be_bytes());
            invalid.extend_from_slice(&payload);
            assert!(decode_motion_datagram(&invalid).is_err());
        }
    }

    #[test]
    fn oversize_bad_kind_and_bad_version_are_rejected_before_decode() {
        assert!(matches!(
            decode_motion_datagram(&[0; 513]),
            Err(ProtocolError::FrameTooLarge { .. })
        ));
        let original = encode_motion_datagram(&sample()).unwrap();
        for offset in 0..4 {
            let mut bytes = original.clone();
            bytes[offset] ^= 1;
            assert!(decode_motion_datagram(&bytes).is_err());
        }
        let mut bytes = original;
        bytes[4..6].copy_from_slice(&600u16.to_be_bytes());
        assert!(matches!(
            decode_motion_datagram(&bytes),
            Err(ProtocolError::FrameTooLarge { .. })
        ));
    }

    #[test]
    fn diagnostic_debug_redacts_authorization_token() {
        let mut value = sample();
        value.token = InputToken([73; 32]);
        let debug = format!("{value:?}");
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains("73, 73"));
    }
}
