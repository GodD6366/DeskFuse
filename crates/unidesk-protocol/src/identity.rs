//! Signed hello identity helpers.
//!
//! `verify_hello` always derives the verification key from the hello itself,
//! checks the device-id binding first, and then uses ed25519-dalek's strict
//! verifier.  There is intentionally no fallback or "insecure" verifier.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::{Hello, PROTOCOL_MAJOR, PROTOCOL_MINOR, ProtocolError};

pub const ED25519_PUBLIC_KEY_SIZE: usize = 32;
pub const ED25519_SIGNATURE_SIZE: usize = 64;
pub const HELLO_NONCE_SIZE: usize = 32;

const HELLO_DOMAIN: &[u8] = b"unidesk/1 hello canonical\0";

/// Return SHA-256(public_key) as the wire-level device id.
#[must_use]
pub fn device_id_for_public_key(public_key: &[u8]) -> [u8; 32] {
    Sha256::digest(public_key).into()
}

/// Build the canonical byte string signed by [`sign_hello`].
///
/// Lengths are encoded as unsigned big-endian u32 values.  The signature field
/// is intentionally absent.
#[must_use]
pub fn canonical_hello_payload(hello: &Hello) -> Vec<u8> {
    fn put_bytes(out: &mut Vec<u8>, value: &[u8]) {
        out.extend_from_slice(
            &u32::try_from(value.len())
                .expect("canonical hello field length must fit in u32")
                .to_be_bytes(),
        );
        out.extend_from_slice(value);
    }

    fn put_string(out: &mut Vec<u8>, value: &str) {
        put_bytes(out, value.as_bytes());
    }

    let mut out = Vec::with_capacity(128);
    out.extend_from_slice(HELLO_DOMAIN);
    out.extend_from_slice(&PROTOCOL_MAJOR.to_be_bytes());
    out.extend_from_slice(&PROTOCOL_MINOR.to_be_bytes());
    put_bytes(&mut out, &hello.device_id);
    put_string(&mut out, &hello.display_name);
    put_bytes(&mut out, &hello.public_key);
    put_bytes(&mut out, &hello.nonce);

    out.extend_from_slice(&hello.capabilities.to_be_bytes());
    out
}

/// Sign a hello's canonical identity payload.
///
/// The returned bytes can be assigned to `Hello::signature`.  The function
/// does not mutate the input, making accidental signing of a stale signature
/// impossible.
pub fn sign_hello(hello: &Hello, signing_key: &SigningKey) -> Result<Vec<u8>, ProtocolError> {
    validate_hello_identity_fields(hello, false)?;
    let expected = device_id_for_public_key(&signing_key.verifying_key().to_bytes());
    if hello.device_id.as_slice() != expected {
        return Err(ProtocolError::IdentityMismatch);
    }
    Ok(signing_key
        .sign(&canonical_hello_payload(hello))
        .to_bytes()
        .to_vec())
}

/// Sign a hello and place the signature in its wire message.
pub fn sign_hello_in_place(
    hello: &mut Hello,
    signing_key: &SigningKey,
) -> Result<(), ProtocolError> {
    hello.signature = sign_hello(hello, signing_key)?;
    Ok(())
}

/// Verify a hello's identity binding and Ed25519 signature.
pub fn verify_hello(hello: &Hello) -> Result<(), ProtocolError> {
    validate_hello_identity_fields(hello, true)?;
    let key_bytes: [u8; ED25519_PUBLIC_KEY_SIZE] = hello
        .public_key
        .as_slice()
        .try_into()
        .map_err(|_| ProtocolError::InvalidPublicKey)?;
    let verifying_key =
        VerifyingKey::from_bytes(&key_bytes).map_err(|_| ProtocolError::InvalidPublicKey)?;
    let signature =
        Signature::from_bytes(&hello.signature.as_slice().try_into().map_err(|_| {
            ProtocolError::InvalidSignatureLength {
                expected: ED25519_SIGNATURE_SIZE,
                actual: hello.signature.len(),
            }
        })?);
    verifying_key
        .verify_strict(&canonical_hello_payload(hello), &signature)
        .map_err(|_| ProtocolError::SignatureVerification)
}

/// Validate the identity-related shape of a hello without checking its
/// cryptographic signature.  This is useful before reporting a malformed
/// peer message; callers that accept a peer must call [`verify_hello`].
pub(crate) fn validate_hello_identity_fields(
    hello: &Hello,
    require_signature: bool,
) -> Result<(), ProtocolError> {
    if hello.device_id.len() != 32 {
        return Err(ProtocolError::InvalidDeviceId {
            expected: 32,
            actual: hello.device_id.len(),
        });
    }
    if hello.public_key.len() != ED25519_PUBLIC_KEY_SIZE {
        return Err(ProtocolError::InvalidPublicKey);
    }
    let expected = device_id_for_public_key(&hello.public_key);
    if hello.device_id.as_slice() != expected {
        return Err(ProtocolError::IdentityMismatch);
    }
    if hello.nonce.len() != HELLO_NONCE_SIZE {
        return Err(ProtocolError::InvalidNonce {
            expected: HELLO_NONCE_SIZE,
            actual: hello.nonce.len(),
        });
    }
    if hello.display_name.is_empty() || hello.display_name.chars().count() > 256 {
        return Err(ProtocolError::InvalidDisplayName);
    }
    if require_signature && hello.signature.len() != ED25519_SIGNATURE_SIZE {
        return Err(ProtocolError::InvalidSignatureLength {
            expected: ED25519_SIGNATURE_SIZE,
            actual: hello.signature.len(),
        });
    }
    Ok(())
}
