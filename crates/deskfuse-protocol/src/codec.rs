use bytes::{Buf, BufMut, Bytes, BytesMut};
use prost::Message as ProstMessage;
use tokio_util::codec::{Decoder, Encoder};

use crate::{Envelope, MAX_FRAME_SIZE, ProtocolError};

/// A length-prefixed protobuf frame codec.
///
/// Frames are encoded as a four-byte unsigned big-endian payload length
/// followed by the protobuf-encoded [`Envelope`].  The length does not include
/// the four-byte prefix itself.
#[derive(Debug, Clone)]
pub struct FrameCodec {
    max_frame_size: usize,
}

impl Default for FrameCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameCodec {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_frame_size: MAX_FRAME_SIZE,
        }
    }

    #[must_use]
    pub const fn max_frame_size(&self) -> usize {
        self.max_frame_size
    }

    /// Encode one complete length-prefixed frame.
    pub fn encode_frame(&self, envelope: &Envelope) -> Result<Bytes, ProtocolError> {
        encode_frame_with_limit(envelope, self.max_frame_size)
    }

    /// Decode one complete length-prefixed frame.
    ///
    /// Unlike the streaming [`Decoder`] implementation, this method requires
    /// all bytes for exactly one frame and reports truncation or trailing data
    /// as typed errors.
    pub fn decode_frame(&self, frame: &[u8]) -> Result<Envelope, ProtocolError> {
        decode_frame_with_limit(frame, self.max_frame_size)
    }
}

impl Encoder<Envelope> for FrameCodec {
    type Error = ProtocolError;

    fn encode(&mut self, item: Envelope, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let encoded = self.encode_frame(&item)?;
        dst.reserve(encoded.len());
        dst.extend_from_slice(&encoded);
        Ok(())
    }
}

impl Decoder for FrameCodec {
    type Item = Envelope;
    type Error = ProtocolError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 4 {
            return Ok(None);
        }

        let payload_len = u32::from_be_bytes([src[0], src[1], src[2], src[3]]) as usize;
        if payload_len > self.max_frame_size {
            src.clear();
            return Err(ProtocolError::FrameTooLarge {
                size: payload_len,
                max: self.max_frame_size,
            });
        }

        let frame_len = 4 + payload_len;
        if src.len() < frame_len {
            src.reserve(frame_len - src.len());
            return Ok(None);
        }

        src.advance(4);
        let payload = src.split_to(payload_len);
        let envelope = decode_envelope_payload(payload.freeze())?;
        envelope.validate()?;
        Ok(Some(envelope))
    }
}

/// Encode a frame using [`MAX_FRAME_SIZE`].
pub fn encode_frame(envelope: &Envelope) -> Result<Bytes, ProtocolError> {
    FrameCodec::new().encode_frame(envelope)
}

/// Decode one frame using [`MAX_FRAME_SIZE`].
pub fn decode_frame(frame: &[u8]) -> Result<Envelope, ProtocolError> {
    FrameCodec::new().decode_frame(frame)
}

fn encode_frame_with_limit(
    envelope: &Envelope,
    max_frame_size: usize,
) -> Result<Bytes, ProtocolError> {
    envelope.validate()?;
    let payload_len = envelope.encoded_len();
    if payload_len > max_frame_size {
        return Err(ProtocolError::FrameTooLarge {
            size: payload_len,
            max: max_frame_size,
        });
    }
    let payload_len_u32 = u32::try_from(payload_len).map_err(|_| ProtocolError::FrameTooLarge {
        size: payload_len,
        max: max_frame_size,
    })?;

    let mut frame = BytesMut::with_capacity(4 + payload_len);
    frame.put_u32(payload_len_u32);
    envelope.encode(&mut frame)?;
    Ok(frame.freeze())
}

fn decode_envelope_payload(payload: impl bytes::Buf) -> Result<Envelope, ProtocolError> {
    // Prost intentionally preserves forward compatibility by ignoring unknown
    // fields. The envelope's oneof is a security-sensitive dispatch point,
    // however, so reject unknown payload tags rather than silently treating a
    // message from a future protocol as an empty/unsupported request.
    let mut bytes = payload;
    let mut raw = Vec::new();
    while bytes.has_remaining() {
        raw.push(bytes.get_u8());
    }
    reject_unknown_payload_tags(&raw)?;
    Ok(Envelope::decode(raw.as_slice())?)
}

fn reject_unknown_payload_tags(payload: &[u8]) -> Result<(), ProtocolError> {
    let mut offset = 0;
    while offset < payload.len() {
        let key = read_varint(payload, &mut offset)
            .ok_or(ProtocolError::MalformedProtobuf("invalid protobuf key"))?;
        let field = key >> 3;
        let wire_type = (key & 0x07) as u8;
        // Payload oneof tags: 10..=18 (control) and 20..=22 (clipboard/input/
        // display app messages, workflow and update control). Future unknown
        // tags are rejected on purpose.
        if field >= 10
            && !(10..=18).contains(&field)
            && !(20..=22).contains(&field)
            && !(30..=34).contains(&field)
            && !(40..=48).contains(&field)
        {
            return Err(ProtocolError::UnknownPayload);
        }
        skip_wire_value(payload, &mut offset, wire_type)?;
    }
    Ok(())
}

fn read_varint(payload: &[u8], offset: &mut usize) -> Option<u64> {
    let mut value = 0_u64;
    for shift in (0..70).step_by(7) {
        let byte = *payload.get(*offset)?;
        *offset += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

fn skip_wire_value(payload: &[u8], offset: &mut usize, wire_type: u8) -> Result<(), ProtocolError> {
    match wire_type {
        0 => {
            read_varint(payload, offset)
                .ok_or(ProtocolError::MalformedProtobuf("invalid protobuf varint"))?;
        }
        1 => {
            let end = offset.checked_add(8).ok_or(ProtocolError::Truncated {
                expected: usize::MAX,
                actual: payload.len(),
            })?;
            if end > payload.len() {
                return Err(ProtocolError::Truncated {
                    expected: end,
                    actual: payload.len(),
                });
            }
            *offset = end;
        }
        2 => {
            let len = read_varint(payload, offset)
                .ok_or(ProtocolError::MalformedProtobuf("invalid protobuf length"))?;
            let len = usize::try_from(len)
                .map_err(|_| ProtocolError::MalformedProtobuf("protobuf length is too large"))?;
            let end = offset.checked_add(len).ok_or(ProtocolError::Truncated {
                expected: usize::MAX,
                actual: payload.len(),
            })?;
            if end > payload.len() {
                return Err(ProtocolError::Truncated {
                    expected: end,
                    actual: payload.len(),
                });
            }
            *offset = end;
        }
        5 => {
            let end = offset.checked_add(4).ok_or(ProtocolError::Truncated {
                expected: usize::MAX,
                actual: payload.len(),
            })?;
            if end > payload.len() {
                return Err(ProtocolError::Truncated {
                    expected: end,
                    actual: payload.len(),
                });
            }
            *offset = end;
        }
        _ => {
            return Err(ProtocolError::MalformedProtobuf(
                "unsupported protobuf wire type",
            ));
        }
    }
    Ok(())
}

fn decode_frame_with_limit(frame: &[u8], max_frame_size: usize) -> Result<Envelope, ProtocolError> {
    if frame.len() < 4 {
        return Err(ProtocolError::Truncated {
            expected: 4,
            actual: frame.len(),
        });
    }

    let payload_len = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    if payload_len > max_frame_size {
        return Err(ProtocolError::FrameTooLarge {
            size: payload_len,
            max: max_frame_size,
        });
    }
    let expected = 4 + payload_len;
    if frame.len() < expected {
        return Err(ProtocolError::Truncated {
            expected,
            actual: frame.len(),
        });
    }
    if frame.len() > expected {
        return Err(ProtocolError::TrailingBytes {
            remaining: frame.len() - expected,
        });
    }

    let envelope = decode_envelope_payload(&frame[4..expected])?;
    envelope.validate()?;
    Ok(envelope)
}
