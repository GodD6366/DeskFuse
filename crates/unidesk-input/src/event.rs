use crate::{DisplayId, Point, Vector2};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, Not};

/// Milliseconds from a monotonic clock. The protocol only carries the scalar;
/// choosing and synchronizing the clock is a transport/platform concern.
pub type Timestamp = u64;

/// A platform-neutral key code. Values may be native scan codes or a protocol
/// assigned code; the input crate treats them as opaque except for Escape.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KeyCode(pub u32);

impl KeyCode {
    /// Escape key code used by the default escape hotkey.
    pub const ESCAPE: Self = Self(0x1b);
    /// Creates a key code from a numeric protocol/native value.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }
}

impl From<u32> for KeyCode {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<&str> for KeyCode {
    fn from(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "escape" | "esc" => Self::ESCAPE,
            _ => Self(0),
        }
    }
}

impl fmt::Display for KeyCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if *self == Self::ESCAPE {
            formatter.write_str("Escape")
        } else {
            write!(formatter, "KeyCode({})", self.0)
        }
    }
}

/// A platform-neutral mouse button code.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ButtonCode(pub u8);

impl ButtonCode {
    /// Primary/left mouse button.
    pub const LEFT: Self = Self(1);
    /// Secondary/right mouse button.
    pub const RIGHT: Self = Self(2);
    /// Middle mouse button.
    pub const MIDDLE: Self = Self(3);

    /// Creates a button code.
    #[must_use]
    pub const fn new(value: u8) -> Self {
        Self(value)
    }
}

impl From<u8> for ButtonCode {
    fn from(value: u8) -> Self {
        Self(value)
    }
}

/// A compact, forward-compatible keyboard modifier bitset.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Modifiers(pub u16);

impl Modifiers {
    /// No modifiers.
    pub const NONE: Self = Self(0);
    /// Shift modifier.
    pub const SHIFT: Self = Self(1 << 0);
    /// Control modifier.
    pub const CTRL: Self = Self(1 << 1);
    /// Alt/Option modifier.
    pub const ALT: Self = Self(1 << 2);
    /// Command/Meta modifier.
    pub const META: Self = Self(1 << 3);
    /// Caps Lock modifier.
    pub const CAPS_LOCK: Self = Self(1 << 4);
    /// Function modifier.
    pub const FN: Self = Self(1 << 5);

    /// Creates a modifier bitset from its wire representation.
    #[must_use]
    pub const fn new(bits: u16) -> Self {
        Self(bits)
    }

    /// Returns whether all bits in `other` are set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Returns the raw wire representation.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }
}

impl BitOr for Modifiers {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Modifiers {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitAnd for Modifiers {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl BitAndAssign for Modifiers {
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

impl Not for Modifiers {
    type Output = Self;

    fn not(self) -> Self::Output {
        Self(!self.0)
    }
}

/// Scroll wheel or trackpad delta.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScrollDelta {
    /// Horizontal scroll delta.
    pub x: f64,
    /// Vertical scroll delta.
    pub y: f64,
}

impl ScrollDelta {
    /// Creates a scroll delta.
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// A timestamped, sequenced pointer observation used by the crossing state
/// machine. It is intentionally an observation rather than an OS event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PointerSample {
    /// Display where the observation was made.
    pub display_id: DisplayId,
    /// Display-local pointer position.
    pub position: Point,
    /// Monotonic timestamp in milliseconds.
    pub timestamp: Timestamp,
    /// Monotonically increasing source sequence number.
    pub sequence: u64,
    /// Velocity in logical pixels per second. A zero vector asks consumers to
    /// derive velocity from consecutive samples when possible.
    pub velocity: Vector2,
}

impl Default for PointerSample {
    fn default() -> Self {
        Self {
            display_id: DisplayId::new(""),
            position: Point::default(),
            timestamp: 0,
            sequence: 0,
            velocity: Vector2::default(),
        }
    }
}

impl PointerSample {
    /// Creates a sample with sequence zero and zero explicit velocity.
    #[must_use]
    pub fn new(display_id: impl Into<DisplayId>, position: Point, timestamp: Timestamp) -> Self {
        Self {
            display_id: display_id.into(),
            position,
            timestamp,
            sequence: 0,
            velocity: Vector2::default(),
        }
    }

    /// Creates a sample directly from x/y coordinates.
    #[must_use]
    pub fn from_xy(
        display_id: impl Into<DisplayId>,
        x: f64,
        y: f64,
        timestamp: Timestamp,
        sequence: u64,
    ) -> Self {
        Self {
            display_id: display_id.into(),
            position: Point::new(x, y),
            timestamp,
            sequence,
            velocity: Vector2::default(),
        }
    }

    /// Returns a copy with a sequence number.
    #[must_use]
    pub const fn with_sequence(mut self, sequence: u64) -> Self {
        self.sequence = sequence;
        self
    }

    /// Returns a copy with an explicit velocity.
    #[must_use]
    pub const fn with_velocity(mut self, velocity: Vector2) -> Self {
        self.velocity = velocity;
        self
    }

    /// Computes velocity from a previous sample, in pixels per second.
    #[must_use]
    pub fn velocity_from(&self, previous: &Self) -> Vector2 {
        let dt = self.timestamp.saturating_sub(previous.timestamp);
        if dt == 0 {
            return Vector2::default();
        }
        let seconds = dt as f64 / 1000.0;
        Vector2::new(
            (self.position.x - previous.position.x) / seconds,
            (self.position.y - previous.position.y) / seconds,
        )
    }
}

/// The input event kind carried over the future protocol.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputEventKind {
    /// Pointer movement.
    Motion,
    /// Mouse button transition.
    Button,
    /// Scroll transition.
    Scroll,
    /// Keyboard key transition.
    Key,
    /// Modifier state update.
    Modifiers,
}

/// Timestamped and sequenced input event. The tagged representation is stable
/// and can be extended with new event kinds without changing existing fields.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputEvent {
    /// Pointer motion.
    Motion {
        /// Display-local pointer position.
        position: Point,
        /// Display receiving the motion.
        display_id: DisplayId,
        /// Optional source velocity.
        velocity: Vector2,
        /// Monotonic sequence number.
        sequence: u64,
        /// Monotonic timestamp in milliseconds.
        timestamp: Timestamp,
    },
    /// Mouse button transition.
    Button {
        /// Button code.
        button: ButtonCode,
        /// Whether the button is now pressed.
        pressed: bool,
        /// Monotonic sequence number.
        sequence: u64,
        /// Monotonic timestamp in milliseconds.
        timestamp: Timestamp,
    },
    /// Scroll transition.
    Scroll {
        /// Scroll delta.
        delta: ScrollDelta,
        /// Monotonic sequence number.
        sequence: u64,
        /// Monotonic timestamp in milliseconds.
        timestamp: Timestamp,
    },
    /// Keyboard transition.
    Key {
        /// Key code.
        key: KeyCode,
        /// Whether the key is now pressed.
        pressed: bool,
        /// Modifiers active with this event.
        modifiers: Modifiers,
        /// Monotonic sequence number.
        sequence: u64,
        /// Monotonic timestamp in milliseconds.
        timestamp: Timestamp,
    },
    /// Modifier state update.
    Modifiers {
        /// Current modifier state.
        modifiers: Modifiers,
        /// Monotonic sequence number.
        sequence: u64,
        /// Monotonic timestamp in milliseconds.
        timestamp: Timestamp,
    },
}

impl InputEvent {
    /// Creates a motion event.
    #[must_use]
    pub fn motion(
        display_id: impl Into<DisplayId>,
        position: Point,
        timestamp: Timestamp,
        sequence: u64,
    ) -> Self {
        Self::Motion {
            position,
            display_id: display_id.into(),
            velocity: Vector2::default(),
            sequence,
            timestamp,
        }
    }

    /// Creates a motion event with velocity.
    #[must_use]
    pub fn motion_with_velocity(
        display_id: impl Into<DisplayId>,
        position: Point,
        velocity: Vector2,
        timestamp: Timestamp,
        sequence: u64,
    ) -> Self {
        Self::Motion {
            position,
            display_id: display_id.into(),
            velocity,
            sequence,
            timestamp,
        }
    }

    /// Creates a button event.
    #[must_use]
    pub const fn button(
        button: ButtonCode,
        pressed: bool,
        timestamp: Timestamp,
        sequence: u64,
    ) -> Self {
        Self::Button {
            button,
            pressed,
            sequence,
            timestamp,
        }
    }

    /// Creates a scroll event.
    #[must_use]
    pub const fn scroll(delta: ScrollDelta, timestamp: Timestamp, sequence: u64) -> Self {
        Self::Scroll {
            delta,
            sequence,
            timestamp,
        }
    }

    /// Creates a key event.
    #[must_use]
    pub const fn key(
        key: KeyCode,
        pressed: bool,
        modifiers: Modifiers,
        timestamp: Timestamp,
        sequence: u64,
    ) -> Self {
        Self::Key {
            key,
            pressed,
            modifiers,
            sequence,
            timestamp,
        }
    }

    /// Creates a modifier update.
    #[must_use]
    pub const fn modifiers(modifiers: Modifiers, timestamp: Timestamp, sequence: u64) -> Self {
        Self::Modifiers {
            modifiers,
            sequence,
            timestamp,
        }
    }

    /// Returns the event's sequence number.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        match self {
            Self::Motion { sequence, .. }
            | Self::Button { sequence, .. }
            | Self::Scroll { sequence, .. }
            | Self::Key { sequence, .. }
            | Self::Modifiers { sequence, .. } => *sequence,
        }
    }

    /// Returns the event's timestamp.
    #[must_use]
    pub const fn timestamp(&self) -> Timestamp {
        match self {
            Self::Motion { timestamp, .. }
            | Self::Button { timestamp, .. }
            | Self::Scroll { timestamp, .. }
            | Self::Key { timestamp, .. }
            | Self::Modifiers { timestamp, .. } => *timestamp,
        }
    }

    /// Returns the event kind.
    #[must_use]
    pub const fn kind(&self) -> InputEventKind {
        match self {
            Self::Motion { .. } => InputEventKind::Motion,
            Self::Button { .. } => InputEventKind::Button,
            Self::Scroll { .. } => InputEventKind::Scroll,
            Self::Key { .. } => InputEventKind::Key,
            Self::Modifiers { .. } => InputEventKind::Modifiers,
        }
    }

    /// Returns the display associated with motion, if any.
    #[must_use]
    pub fn display_id(&self) -> Option<&DisplayId> {
        match self {
            Self::Motion { display_id, .. } => Some(display_id),
            _ => None,
        }
    }

    /// Returns true for a pressed Escape key with the supplied modifier set.
    #[must_use]
    pub fn is_escape(&self, required_modifiers: Modifiers) -> bool {
        matches!(
            self,
            Self::Key {
                key: KeyCode::ESCAPE,
                pressed: true,
                modifiers,
                ..
            } if modifiers.contains(required_modifiers)
        )
    }
}
