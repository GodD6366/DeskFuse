use crate::DisplayId;
use getrandom::fill;
use std::fmt;

/// Number of bytes in a lease token.
pub const LEASE_TOKEN_LEN: usize = 32;

/// Error returned when a lease token cannot be generated or decoded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeaseTokenError {
    /// The operating system's cryptographically secure random source failed.
    RandomUnavailable,
    /// Wire data was not exactly [`LEASE_TOKEN_LEN`] bytes long.
    InvalidLength {
        /// Required byte length.
        expected: usize,
        /// Received byte length.
        actual: usize,
    },
}

impl fmt::Display for LeaseTokenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RandomUnavailable => {
                formatter.write_str("cryptographically secure random source unavailable")
            }
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "lease token must contain {expected} bytes (got {actual})"
                )
            }
        }
    }
}

impl std::error::Error for LeaseTokenError {}

/// Opaque lease token used to authenticate renew/revoke calls.
///
/// Tokens are generated from the operating system's CSPRNG and are never
/// formatted as their contents. The only way to obtain wire bytes is the
/// explicit [`LeaseToken::to_wire`] conversion; callers must treat that value
/// as a secret and must not log it.
#[derive(Clone, Eq)]
pub struct LeaseToken([u8; LEASE_TOKEN_LEN]);

impl LeaseToken {
    /// Generates a fresh 256-bit token from the operating system CSPRNG.
    pub fn generate() -> Result<Self, LeaseTokenError> {
        let mut bytes = [0_u8; LEASE_TOKEN_LEN];
        fill(&mut bytes).map_err(|_| LeaseTokenError::RandomUnavailable)?;
        Ok(Self(bytes))
    }

    /// Creates a token from exactly-sized wire bytes.
    ///
    /// This is intended for parsing an authenticated protocol message, not
    /// for issuing a new lease. Issuers must use [`LeaseToken::generate`].
    #[must_use]
    pub const fn from_wire(bytes: [u8; LEASE_TOKEN_LEN]) -> Self {
        Self(bytes)
    }

    /// Parses wire bytes, rejecting lengths other than 32 bytes.
    pub fn try_from_wire(bytes: &[u8]) -> Result<Self, LeaseTokenError> {
        let bytes: [u8; LEASE_TOKEN_LEN] =
            bytes
                .try_into()
                .map_err(|_| LeaseTokenError::InvalidLength {
                    expected: LEASE_TOKEN_LEN,
                    actual: bytes.len(),
                })?;
        Ok(Self::from_wire(bytes))
    }

    /// Returns the fixed-size wire representation.
    ///
    /// The returned bytes are secret material and must not be logged.
    #[must_use]
    pub const fn to_wire(&self) -> [u8; LEASE_TOKEN_LEN] {
        self.0
    }

    /// Returns the fixed wire length without exposing token contents.
    #[must_use]
    pub const fn wire_len(&self) -> usize {
        LEASE_TOKEN_LEN
    }
}

impl PartialEq for LeaseToken {
    fn eq(&self, other: &Self) -> bool {
        // Accumulate all differences instead of returning at the first
        // mismatch, avoiding a timing signal about the matching prefix.
        let mut difference = 0_u8;
        for (&left, &right) in self.0.iter().zip(other.0.iter()) {
            difference |= left ^ right;
        }
        difference == 0
    }
}

impl fmt::Debug for LeaseToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

/// A time-bounded right for one controller to route input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlLease {
    /// Device holding the lease.
    pub owner: DisplayId,
    /// Opaque token proving ownership. Its debug representation is redacted.
    pub token: LeaseToken,
    /// Grant timestamp in monotonic milliseconds.
    pub granted_at: u64,
    /// Expiry timestamp in monotonic milliseconds (exclusive).
    pub expires_at: u64,
}

impl ControlLease {
    /// Returns the remaining TTL at `now`, saturating at zero.
    #[must_use]
    pub fn remaining_ms(&self, now: u64) -> u64 {
        self.expires_at.saturating_sub(now)
    }

    /// Returns whether the lease has expired at `now`.
    #[must_use]
    pub const fn is_expired_at(&self, now: u64) -> bool {
        now >= self.expires_at
    }

    /// Returns whether the lease is still valid at `now`.
    #[must_use]
    pub fn is_valid_at(&self, now: u64) -> bool {
        !self.is_expired_at(now)
    }

    /// Returns whether the lease token matches.
    #[must_use]
    pub fn authenticates(&self, owner: &DisplayId, token: &LeaseToken) -> bool {
        &self.owner == owner && &self.token == token
    }
}

/// Errors from exclusive lease operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LeaseError {
    /// Another owner currently holds a non-expired lease.
    AlreadyHeld {
        /// Current lease holder.
        owner: DisplayId,
    },
    /// No non-expired lease exists.
    NotHeld,
    /// Owner/token pair does not match the current lease.
    NotOwner,
    /// A zero TTL is not allowed.
    InvalidTtl,
    /// The OS CSPRNG could not produce a secure token.
    TokenGenerationFailed,
}

impl fmt::Display for LeaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyHeld { owner } => write!(formatter, "control lease is held by {owner}"),
            Self::NotHeld => formatter.write_str("control lease is not held"),
            Self::NotOwner => formatter.write_str("control lease owner or token does not match"),
            Self::InvalidTtl => formatter.write_str("control lease TTL must be greater than zero"),
            Self::TokenGenerationFailed => {
                formatter.write_str("could not generate a secure control lease token")
            }
        }
    }
}

impl std::error::Error for LeaseError {}

/// Single-owner control lease manager. It is intentionally clock-free: callers
/// provide monotonic timestamps so tests and transport adapters remain
/// deterministic.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ControlLeaseManager {
    lease: Option<ControlLease>,
}

impl ControlLeaseManager {
    /// Creates an empty lease manager.
    #[must_use]
    pub const fn new() -> Self {
        Self { lease: None }
    }

    /// Returns the current non-expired lease, if any.
    #[must_use]
    pub fn current(&self, now: u64) -> Option<&ControlLease> {
        self.lease
            .as_ref()
            .filter(|lease| !lease.is_expired_at(now))
    }

    /// Returns a copy of the current non-expired lease.
    #[must_use]
    pub fn snapshot(&self, now: u64) -> Option<ControlLease> {
        self.current(now).cloned()
    }

    /// Returns whether a non-expired lease is held.
    #[must_use]
    pub fn is_held(&self, now: u64) -> bool {
        self.current(now).is_some()
    }

    /// Attempts to grant the sole lease to `owner`.
    pub fn acquire(
        &mut self,
        owner: impl Into<DisplayId>,
        now: u64,
        ttl_ms: u64,
    ) -> Result<ControlLease, LeaseError> {
        if ttl_ms == 0 {
            return Err(LeaseError::InvalidTtl);
        }
        if let Some(current) = self.current(now) {
            return Err(LeaseError::AlreadyHeld {
                owner: current.owner.clone(),
            });
        }
        let token = LeaseToken::generate().map_err(|_| LeaseError::TokenGenerationFailed)?;
        let lease = ControlLease {
            owner: owner.into(),
            token,
            granted_at: now,
            expires_at: now.saturating_add(ttl_ms),
        };
        self.lease = Some(lease.clone());
        Ok(lease)
    }

    /// Renews the lease if owner and token match. Renewal is relative to the
    /// supplied `now`, never to the old expiry, which prevents extending an
    /// already expired lease.
    pub fn renew(
        &mut self,
        owner: impl Into<DisplayId>,
        token: &LeaseToken,
        now: u64,
        ttl_ms: u64,
    ) -> Result<ControlLease, LeaseError> {
        if ttl_ms == 0 {
            return Err(LeaseError::InvalidTtl);
        }
        let owner = owner.into();
        let Some(current) = self.lease.as_ref() else {
            return Err(LeaseError::NotHeld);
        };
        if current.is_expired_at(now) {
            self.lease = None;
            return Err(LeaseError::NotHeld);
        }
        if !current.authenticates(&owner, token) {
            return Err(LeaseError::NotOwner);
        }
        let renewed = ControlLease {
            owner,
            token: token.clone(),
            granted_at: current.granted_at,
            expires_at: now.saturating_add(ttl_ms),
        };
        self.lease = Some(renewed.clone());
        Ok(renewed)
    }

    /// Revokes a lease using owner and token.
    pub fn revoke(
        &mut self,
        owner: impl Into<DisplayId>,
        token: &LeaseToken,
        now: u64,
    ) -> Result<ControlLease, LeaseError> {
        let owner = owner.into();
        let Some(current) = self.lease.as_ref() else {
            return Err(LeaseError::NotHeld);
        };
        if current.is_expired_at(now) {
            self.lease = None;
            return Err(LeaseError::NotHeld);
        }
        if !current.authenticates(&owner, token) {
            return Err(LeaseError::NotOwner);
        }
        self.lease.take().ok_or(LeaseError::NotHeld)
    }

    /// Revokes the current lease regardless of owner. This is intended for a
    /// trusted disconnect/session teardown path.
    pub fn revoke_all(&mut self) -> Option<ControlLease> {
        self.lease.take()
    }

    /// Releases a lease using owner and token. This is an ergonomic alias for
    /// [`ControlLeaseManager::revoke`].
    pub fn release(
        &mut self,
        owner: impl Into<DisplayId>,
        token: &LeaseToken,
        now: u64,
    ) -> Result<ControlLease, LeaseError> {
        self.revoke(owner, token, now)
    }

    /// Expires a lease if its TTL elapsed. Returns the expired lease.
    pub fn expire(&mut self, now: u64) -> Option<ControlLease> {
        if self
            .lease
            .as_ref()
            .is_some_and(|lease| lease.is_expired_at(now))
        {
            self.lease.take()
        } else {
            None
        }
    }

    /// Alias for [`ControlLeaseManager::expire`].
    pub fn tick(&mut self, now: u64) -> Option<ControlLease> {
        self.expire(now)
    }
}

/// Short alias used by transport adapters.
pub type LeaseManager = ControlLeaseManager;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_random_fixed_length_and_redacted() {
        let first = LeaseToken::generate().expect("CSPRNG");
        let second = LeaseToken::generate().expect("CSPRNG");
        assert_ne!(first, second);
        assert_eq!(first.wire_len(), LEASE_TOKEN_LEN);
        assert_eq!(first.to_wire().len(), LEASE_TOKEN_LEN);
        assert_eq!(format!("{first:?}"), "<redacted>");
        assert!(
            !format!("{first:?}").contains(
                &first
                    .to_wire()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            )
        );
    }

    #[test]
    fn lease_is_exclusive_and_expires() {
        let mut manager = ControlLeaseManager::new();
        let first = manager.acquire("main", 100, 50).expect("lease");
        assert_eq!(
            manager.acquire("peer", 110, 50),
            Err(LeaseError::AlreadyHeld {
                owner: DisplayId::from("main")
            })
        );
        assert!(manager.current(149).is_some());
        assert!(manager.current(150).is_none());
        assert_eq!(manager.expire(151), Some(first));
        let second = manager.acquire("peer", 152, 10).expect("new lease");
        assert_eq!(second.owner, DisplayId::from("peer"));
    }

    #[test]
    fn wrong_owner_and_token_cannot_renew_or_revoke() {
        let mut manager = ControlLeaseManager::new();
        let lease = manager.acquire("main", 0, 100).expect("lease");
        let wrong_token = LeaseToken::generate().expect("CSPRNG");
        assert_eq!(
            manager.renew("peer", &lease.token, 10, 100),
            Err(LeaseError::NotOwner)
        );
        assert_eq!(
            manager.renew("main", &wrong_token, 10, 100),
            Err(LeaseError::NotOwner)
        );
        assert_eq!(
            manager.revoke("main", &wrong_token, 10),
            Err(LeaseError::NotOwner)
        );
        assert!(manager.current(10).is_some());
    }

    #[test]
    fn wire_conversion_requires_exact_length() {
        let token = LeaseToken::generate().expect("CSPRNG");
        let wire = token.to_wire();
        assert_eq!(LeaseToken::try_from_wire(&wire), Ok(token));
        assert_eq!(
            LeaseToken::try_from_wire(&wire[..31]),
            Err(LeaseTokenError::InvalidLength {
                expected: LEASE_TOKEN_LEN,
                actual: 31,
            })
        );
    }
}
