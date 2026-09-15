use thiserror::Error;

/// Errors raised while validating, encoding, or decoding a protocol frame.
#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("frame is too large: {size} bytes (maximum {max} bytes)")]
    FrameTooLarge { size: usize, max: usize },

    #[error("truncated frame: expected {expected} bytes, received {actual} bytes")]
    Truncated { expected: usize, actual: usize },

    #[error("frame has {remaining} trailing bytes after the length-delimited payload")]
    TrailingBytes { remaining: usize },

    #[error("malformed protobuf envelope: {0}")]
    Decode(#[from] prost::DecodeError),

    #[error("could not encode protobuf envelope: {0}")]
    Encode(#[from] prost::EncodeError),

    #[error("malformed protobuf payload: {0}")]
    MalformedProtobuf(&'static str),

    #[error("I/O error while framing protocol data: {0}")]
    Io(#[from] std::io::Error),

    #[error("unsupported protocol major version {received}; supported major is {supported}")]
    IncompatibleMajor { received: u32, supported: u32 },

    #[error("protocol envelope has no known payload")]
    UnknownPayload,

    #[error("invalid request id: expected exactly {expected} bytes, got {actual}")]
    InvalidRequestId { expected: usize, actual: usize },

    #[error("invalid device id: expected exactly {expected} bytes, got {actual}")]
    InvalidDeviceId { expected: usize, actual: usize },

    #[error("device id does not equal SHA-256(public_key)")]
    IdentityMismatch,

    #[error("invalid Ed25519 public key")]
    InvalidPublicKey,

    #[error("invalid Ed25519 signature: expected exactly {expected} bytes, got {actual}")]
    InvalidSignatureLength { expected: usize, actual: usize },

    #[error("Ed25519 signature verification failed")]
    SignatureVerification,

    #[error("invalid nonce: expected exactly {expected} bytes, got {actual}")]
    InvalidNonce { expected: usize, actual: usize },

    #[error("invalid display name")]
    InvalidDisplayName,

    #[error("invalid capability")]
    InvalidCapability,

    #[error("invalid payload: {0}")]
    InvalidPayload(&'static str),

    #[error("core conversion failed: {0}")]
    CoreConversion(String),
}
