//! Platform independent input-routing domain types.
//!
//! This crate deliberately does not inject events into an operating system. It
//! owns the geometry, routing, state, batching, and lease rules that a platform
//! adapter can use to perform injection later.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations, missing_docs)]

mod batch;
pub mod bridge;
mod crossing;
mod event;
mod geometry;
pub mod layout;
mod lease;
mod motion;
pub mod quality;
pub mod sharing;
mod topology;

pub use batch::{
    BatchError, BatchPolicy, BatchStats, DropReason, InputBatch, InputBatcher, PushOutcome,
    SequenceGap, SequencePolicy, same_motion_display,
};
pub use crossing::{
    ConnectionState, CrossingAction, CrossingConfig, CrossingEvent, CrossingReason,
    CrossingStateMachine, EscapeHotkey, ReturnReason,
};
pub use event::{
    ButtonCode, InputEvent, InputEventKind, KeyCode, Modifiers, PointerSample, ScrollDelta,
    Timestamp,
};
pub use geometry::{Edge, Point, Rect, Size, Vector2};
pub use lease::{
    ControlLease, ControlLeaseManager, LEASE_TOKEN_LEN, LeaseError, LeaseManager, LeaseToken,
    LeaseTokenError,
};
pub use motion::{FUTURE_MOTION_TTL_MS, MAX_FUTURE_STATE_EPOCH_DELTA, MotionReceiver};
pub use quality::{
    current_report, normalize_pointer, retest, DisplayGeometry, InputQualityReport,
    NormalizedMotion, QualityHistory, QualitySuggestion,
};
pub use topology::{DevicePlacement, DeviceRole, DisplayId, EdgeMapping, Topology, TopologyError};
