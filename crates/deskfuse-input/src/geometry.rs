use serde::{Deserialize, Serialize};
use std::fmt;

/// A point in display-local logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: f64,
    /// Vertical coordinate.
    pub y: f64,
}

impl Point {
    /// Creates a point.
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Returns the Euclidean distance to another point.
    #[must_use]
    pub fn distance(self, other: Self) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        dx.hypot(dy)
    }
}

/// A two-dimensional size in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Size {
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

impl Size {
    /// Creates a size.
    #[must_use]
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

/// A two-dimensional vector, used for pointer velocity and scroll deltas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vector2 {
    /// Horizontal component.
    pub x: f64,
    /// Vertical component.
    pub y: f64,
}

impl Vector2 {
    /// Creates a vector.
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// Returns the vector magnitude.
    #[must_use]
    pub fn magnitude(self) -> f64 {
        self.x.hypot(self.y)
    }
}

/// An axis-aligned display rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    /// Left coordinate in the topology's coordinate space.
    pub x: f64,
    /// Top coordinate in the topology's coordinate space.
    pub y: f64,
    /// Width. Must be positive for a valid rectangle.
    pub width: f64,
    /// Height. Must be positive for a valid rectangle.
    pub height: f64,
}

impl Default for Rect {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }
}

impl Rect {
    /// Creates an axis-aligned rectangle.
    #[must_use]
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Right edge coordinate.
    #[must_use]
    pub const fn right(self) -> f64 {
        self.x + self.width
    }

    /// Bottom edge coordinate.
    #[must_use]
    pub const fn bottom(self) -> f64 {
        self.y + self.height
    }

    /// Returns whether a point is inside the rectangle, including its edges.
    #[must_use]
    pub fn contains(self, point: Point) -> bool {
        point.x >= self.x
            && point.x <= self.right()
            && point.y >= self.y
            && point.y <= self.bottom()
    }

    /// Returns the length of the overlap between two intervals.
    #[must_use]
    pub fn overlap(a_start: f64, a_end: f64, b_start: f64, b_end: f64) -> f64 {
        (a_end.min(b_end) - a_start.max(b_start)).max(0.0)
    }

    /// Returns the coordinate range along an edge which can be mapped to an
    /// adjacent rectangle. The range is expressed in the axis of the edge.
    #[must_use]
    pub fn edge_span(self, edge: Edge) -> (f64, f64) {
        match edge {
            Edge::Left | Edge::Right => (self.y, self.bottom()),
            Edge::Top | Edge::Bottom => (self.x, self.right()),
        }
    }

    /// Returns the signed distance from a point to an edge. A positive value
    /// means the point is inside the rectangle, while a negative value means
    /// it is beyond the edge.
    #[must_use]
    pub fn signed_distance_to_edge(self, point: Point, edge: Edge) -> f64 {
        match edge {
            Edge::Left => point.x - self.x,
            Edge::Right => self.right() - point.x,
            Edge::Top => point.y - self.y,
            Edge::Bottom => self.bottom() - point.y,
        }
    }

    /// Returns a point at a distance inside the requested edge, at the given
    /// along-edge coordinate.
    #[must_use]
    pub fn point_on_edge(self, edge: Edge, along: f64, inward_distance: f64) -> Point {
        match edge {
            Edge::Left => Point::new(self.x + inward_distance, along),
            Edge::Right => Point::new(self.right() - inward_distance, along),
            Edge::Top => Point::new(along, self.y + inward_distance),
            Edge::Bottom => Point::new(along, self.bottom() - inward_distance),
        }
    }
}

/// One of the four sides of a display.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    /// Left side.
    Left,
    /// Right side.
    Right,
    /// Top side.
    Top,
    /// Bottom side.
    Bottom,
}

impl Edge {
    /// Returns the opposite edge.
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
        }
    }

    /// Returns whether the edge is horizontal.
    #[must_use]
    pub const fn is_horizontal(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }

    /// Returns the outward unit vector for this edge.
    #[must_use]
    pub const fn outward(self) -> Vector2 {
        match self {
            Self::Left => Vector2::new(-1.0, 0.0),
            Self::Right => Vector2::new(1.0, 0.0),
            Self::Top => Vector2::new(0.0, -1.0),
            Self::Bottom => Vector2::new(0.0, 1.0),
        }
    }
}

impl fmt::Display for Edge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Top => "top",
            Self::Bottom => "bottom",
        })
    }
}
