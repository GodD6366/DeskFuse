//! 相对移动 datagram。前缀与绝对坐标包分开，旧解码器会拒绝未知种类。

use deskfuse_core::{InputSessionId, InputToken, RelativeMotionDatagram};
use prost::Message;

use crate::{ProtocolError, motion::MAX_MOTION_DATAGRAM_SIZE};

// DF (DeskFuse), R (relative), 帧版本 1。
const PREFIX: [u8; 4] = [b'D', b'F', b'R', 1];
const HEADER_SIZE: usize = 6;

#[derive(Clone, PartialEq, Message)]
struct RelativeMotionPacket {
    #[prost(bytes = "vec", tag = "1")]
    session_id: Vec<u8>,
    #[prost(bytes = "vec", tag = "2")]
    token: Vec<u8>,
    #[prost(uint64, tag = "3")]
    state_epoch: u64,
    #[prost(uint64, tag = "4")]
    motion_seq: u64,
    #[prost(double, tag = "5")]
    dx: f64,
    #[prost(double, tag = "6")]
    dy: f64,
}

/// 编码一份已通过校验的相对位移。
pub fn encode_relative_motion_datagram(
    motion: &RelativeMotionDatagram,
) -> Result<Vec<u8>, ProtocolError> {
    motion
        .validate()
        .map_err(|_| ProtocolError::InvalidPayload("invalid relative motion datagram"))?;
    let packet = RelativeMotionPacket {
        session_id: motion.session_id.0.as_bytes().to_vec(),
        token: motion.token.0.to_vec(),
        state_epoch: motion.state_epoch,
        motion_seq: motion.motion_seq,
        dx: motion.dx,
        dy: motion.dy,
    };
    let payload_size = packet.encoded_len();
    check_size(HEADER_SIZE + payload_size)?;
    let mut bytes = Vec::with_capacity(HEADER_SIZE + payload_size);
    bytes.extend_from_slice(&PREFIX);
    bytes.extend_from_slice(&(payload_size as u16).to_be_bytes());
    packet.encode(&mut bytes)?;
    Ok(bytes)
}

/// 解码一整包相对位移。截断、拼接和绝对坐标包都会被拒绝。
pub fn decode_relative_motion_datagram(
    bytes: &[u8],
) -> Result<RelativeMotionDatagram, ProtocolError> {
    check_size(bytes.len())?;
    if bytes.len() < HEADER_SIZE {
        return Err(ProtocolError::Truncated {
            expected: HEADER_SIZE,
            actual: bytes.len(),
        });
    }
    if bytes[..4] != PREFIX {
        return Err(ProtocolError::InvalidPayload(
            "unknown relative motion kind or version",
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
    let packet = RelativeMotionPacket::decode(&bytes[HEADER_SIZE..])?;
    let session_id = uuid::Uuid::from_slice(&packet.session_id)
        .map_err(|_| ProtocolError::InvalidPayload("invalid relative motion session ID"))?;
    let token: [u8; 32] = packet.token.as_slice().try_into().map_err(|_| {
        ProtocolError::InvalidPayload("invalid relative motion token length")
    })?;
    let motion = RelativeMotionDatagram {
        session_id: InputSessionId(session_id),
        token: InputToken(token),
        state_epoch: packet.state_epoch,
        motion_seq: packet.motion_seq,
        dx: packet.dx,
        dy: packet.dy,
    };
    motion
        .validate()
        .map_err(|_| ProtocolError::InvalidPayload("invalid relative motion datagram"))?;
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
    use crate::motion::{decode_motion_datagram, encode_motion_datagram};
    use deskfuse_core::{DisplayId, MotionDatagram, MotionPosition};

    fn sample() -> RelativeMotionDatagram {
        RelativeMotionDatagram {
            session_id: InputSessionId::new(),
            token: InputToken([7; 32]),
            state_epoch: 3,
            motion_seq: 9,
            dx: 12.0,
            dy: -4.5,
        }
    }

    #[test]
    fn relative_packet_roundtrips_and_rejects_invalid_deltas() {
        let value = sample();
        let encoded = encode_relative_motion_datagram(&value).unwrap();
        assert_eq!(encoded[..4], PREFIX);
        assert_eq!(decode_relative_motion_datagram(&encoded).unwrap(), value);
        let mut invalid = value.clone();
        invalid.dx = f64::NAN;
        assert!(encode_relative_motion_datagram(&invalid).is_err());
        invalid.dx = 32769.0;
        assert!(encode_relative_motion_datagram(&invalid).is_err());
    }

    #[test]
    fn relative_and_absolute_packets_do_not_decode_as_each_other() {
        let relative = encode_relative_motion_datagram(&sample()).unwrap();
        assert!(decode_motion_datagram(&relative).is_err());
        let absolute = encode_motion_datagram(&MotionDatagram {
            session_id: InputSessionId::new(),
            token: InputToken([7; 32]),
            state_epoch: 1,
            position: MotionPosition {
                target_display: DisplayId("display".into()),
                topology_generation: 1,
                motion_seq: 1,
                x: 1.0,
                y: 2.0,
            },
        })
        .unwrap();
        assert!(decode_relative_motion_datagram(&absolute).is_err());
        for len in 0..relative.len() {
            assert!(decode_relative_motion_datagram(&relative[..len]).is_err());
        }
        let mut extra = relative;
        extra.push(0);
        assert!(matches!(
            decode_relative_motion_datagram(&extra),
            Err(ProtocolError::TrailingBytes { .. })
        ));
    }
}
