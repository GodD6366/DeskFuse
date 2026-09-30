use crate::{Edge, Point, Rect};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Stable identifier for a physical or logical display/device.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct DisplayId(pub String);

impl DisplayId {
    /// Creates an identifier from any string-like value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for DisplayId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for DisplayId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<u64> for DisplayId {
    fn from(value: u64) -> Self {
        Self(value.to_string())
    }
}

impl fmt::Display for DisplayId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The role a device has in a topology.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRole {
    /// The one device that owns the local pointer and control lease.
    Controller,
    /// A device which can receive routed pointer and keyboard input.
    Controlled,
}

/// A display's placement and role in the shared topology.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DevicePlacement {
    /// Stable display identifier.
    pub display_id: DisplayId,
    /// Rectangle in the shared topology coordinate space.
    pub rect: Rect,
    /// Whether this device is the controller or a controlled peer.
    pub role: DeviceRole,
}

impl DevicePlacement {
    /// Creates a placement.
    #[must_use]
    pub fn new(display_id: impl Into<DisplayId>, rect: Rect, role: DeviceRole) -> Self {
        Self {
            display_id: display_id.into(),
            rect,
            role,
        }
    }

    /// Creates a controller placement.
    #[must_use]
    pub fn controller(display_id: impl Into<DisplayId>, rect: Rect) -> Self {
        Self::new(display_id, rect, DeviceRole::Controller)
    }

    /// Creates a controlled-peer placement.
    #[must_use]
    pub fn controlled(display_id: impl Into<DisplayId>, rect: Rect) -> Self {
        Self::new(display_id, rect, DeviceRole::Controlled)
    }

    /// Alias for [`DevicePlacement::controlled`].
    #[must_use]
    pub fn peer(display_id: impl Into<DisplayId>, rect: Rect) -> Self {
        Self::controlled(display_id, rect)
    }
}

/// A directed mapping between two adjacent display edges.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeMapping {
    /// Source display.
    pub from_display: DisplayId,
    /// Edge on the source display.
    pub from_edge: Edge,
    /// Target display.
    pub to_display: DisplayId,
    /// Edge on the target display where the pointer appears.
    pub to_edge: Edge,
    /// Start of the shared segment in topology coordinates.
    pub overlap_start: f64,
    /// End of the shared segment in topology coordinates.
    pub overlap_end: f64,
    /// Source edge's full along-edge start coordinate.
    pub source_span_start: f64,
    /// Source edge's full along-edge end coordinate.
    pub source_span_end: f64,
    /// Target edge's full along-edge start coordinate.
    pub target_span_start: f64,
    /// Target edge's full along-edge end coordinate.
    pub target_span_end: f64,
    /// Start of the overlapping segment on the target edge.
    pub target_overlap_start: f64,
    /// End of the overlapping segment on the target edge.
    pub target_overlap_end: f64,
    /// Target display rectangle, retained so adapters can map a point without
    /// carrying a second topology lookup.
    pub target_rect: Rect,
}

impl EdgeMapping {
    /// Returns the shared segment length.
    #[must_use]
    pub fn overlap_length(&self) -> f64 {
        self.overlap_end - self.overlap_start
    }

    /// Returns the source display id.
    #[must_use]
    pub fn source_display(&self) -> &DisplayId {
        &self.from_display
    }

    /// Returns the target display id.
    #[must_use]
    pub fn target_display(&self) -> &DisplayId {
        &self.to_display
    }

    /// Returns the source edge.
    #[must_use]
    pub const fn source_edge(&self) -> Edge {
        self.from_edge
    }

    /// Returns the target edge.
    #[must_use]
    pub const fn target_edge(&self) -> Edge {
        self.to_edge
    }

    /// Maps an along-edge coordinate from source to target, preserving its
    /// relative position within the overlapping segment.
    #[must_use]
    pub fn map_along(&self, source_along: f64) -> f64 {
        let source_len = self.overlap_end - self.overlap_start;
        if source_len <= f64::EPSILON {
            return (self.target_span_start + self.target_span_end) / 2.0;
        }
        let ratio = ((source_along - self.overlap_start) / source_len).clamp(0.0, 1.0);
        self.target_overlap_start + ratio * (self.target_overlap_end - self.target_overlap_start)
    }

    /// Maps a source point to the target edge. Values outside the shared
    /// segment are clamped to the target edge's endpoints.
    #[must_use]
    pub fn map_position(&self, source_point: Point) -> Point {
        let source_along = if self.from_edge.is_horizontal() {
            source_point.x
        } else {
            source_point.y
        };
        let target_along = self.map_along(source_along);
        self.target_rect
            .point_on_edge(self.to_edge, target_along, 0.0)
    }

    /// Maps a point on a source rectangle to a point just inside the target
    /// rectangle, using the actual target rectangle's geometry.
    #[must_use]
    pub fn map_point_with_rects(
        &self,
        source_point: Point,
        source_rect: Rect,
        target_rect: Rect,
        inward_distance: f64,
    ) -> Point {
        let source_along = if self.from_edge.is_horizontal() {
            source_point.x
        } else {
            source_point.y
        };
        let target_along = self.map_along(source_along);
        let _ = source_rect;
        target_rect.point_on_edge(self.to_edge, target_along, inward_distance.max(0.0))
    }
}

/// Errors from constructing or editing a topology.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TopologyError {
    /// An identifier already has a placement.
    DuplicateDisplay(DisplayId),
    /// The topology must have exactly one controller.
    InvalidControllerCount,
    /// A display was not found.
    UnknownDisplay(DisplayId),
    /// A rectangle has a non-positive or non-finite dimension.
    InvalidRectangle(DisplayId),
}

impl fmt::Display for TopologyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateDisplay(id) => write!(formatter, "display {id} is already present"),
            Self::InvalidControllerCount => {
                formatter.write_str("topology must have exactly one controller")
            }
            Self::UnknownDisplay(id) => write!(formatter, "unknown display {id}"),
            Self::InvalidRectangle(id) => {
                write!(formatter, "display {id} has an invalid rectangle")
            }
        }
    }
}

impl std::error::Error for TopologyError {}

/// The set of displays and their geometric adjacency rules.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Topology {
    /// Display which owns the local input device.
    pub local_display: DisplayId,
    placements: BTreeMap<DisplayId, DevicePlacement>,
    /// Distance tolerance when deciding whether two sides touch.
    pub adjacency_tolerance: f64,
    /// Minimum non-zero overlap required for an edge mapping. This excludes
    /// point-only corner contacts, which are intentionally ambiguous.
    pub minimum_overlap: f64,
}

impl Topology {
    /// Creates an empty topology associated with a local display id.
    #[must_use]
    pub fn new(local_display: impl Into<DisplayId>) -> Self {
        Self {
            local_display: local_display.into(),
            placements: BTreeMap::new(),
            adjacency_tolerance: 0.001,
            minimum_overlap: 0.001,
        }
    }

    /// Creates a topology from placements, validating controller uniqueness.
    pub fn with_placements(
        local_display: impl Into<DisplayId>,
        placements: impl IntoIterator<Item = DevicePlacement>,
    ) -> Result<Self, TopologyError> {
        let mut topology = Self::new(local_display);
        for placement in placements {
            topology.add(placement)?;
        }
        topology.validate_controller_count()?;
        Ok(topology)
    }

    /// Adds a placement, rejecting duplicate ids and invalid rectangles.
    pub fn add(&mut self, placement: DevicePlacement) -> Result<(), TopologyError> {
        if !placement.rect.width.is_finite()
            || !placement.rect.height.is_finite()
            || placement.rect.width <= 0.0
            || placement.rect.height <= 0.0
        {
            return Err(TopologyError::InvalidRectangle(placement.display_id));
        }
        if self.placements.contains_key(&placement.display_id) {
            return Err(TopologyError::DuplicateDisplay(placement.display_id));
        }
        self.placements
            .insert(placement.display_id.clone(), placement);
        Ok(())
    }

    /// Alias for [`Topology::add`].
    pub fn add_placement(&mut self, placement: DevicePlacement) -> Result<(), TopologyError> {
        self.add(placement)
    }

    /// Replaces or inserts a placement by id.
    pub fn upsert(&mut self, placement: DevicePlacement) -> Result<(), TopologyError> {
        if !placement.rect.width.is_finite()
            || !placement.rect.height.is_finite()
            || placement.rect.width <= 0.0
            || placement.rect.height <= 0.0
        {
            return Err(TopologyError::InvalidRectangle(placement.display_id));
        }
        self.placements
            .insert(placement.display_id.clone(), placement);
        Ok(())
    }

    /// Returns a placement by id.
    #[must_use]
    pub fn placement(&self, display_id: &DisplayId) -> Option<&DevicePlacement> {
        self.placements.get(display_id)
    }

    /// Returns all placements in stable id order.
    pub fn placements(&self) -> impl Iterator<Item = &DevicePlacement> {
        self.placements.values()
    }

    /// Returns all placements as a vector for protocol serialization callers.
    #[must_use]
    pub fn placement_list(&self) -> Vec<DevicePlacement> {
        self.placements.values().cloned().collect()
    }

    /// Validates that there is one and only one controller.
    pub fn validate_controller_count(&self) -> Result<(), TopologyError> {
        let count = self
            .placements
            .values()
            .filter(|placement| placement.role == DeviceRole::Controller)
            .count();
        if count == 1 {
            Ok(())
        } else {
            Err(TopologyError::InvalidControllerCount)
        }
    }

    /// Returns all directed mappings leaving a display on an edge.
    #[must_use]
    pub fn mappings_from(&self, display_id: &DisplayId, edge: Edge) -> Vec<EdgeMapping> {
        let Some(source) = self.placements.get(display_id) else {
            return Vec::new();
        };
        self.placements
            .values()
            .filter(|target| target.display_id != *display_id)
            .filter_map(|target| self.mapping_between(source, target, edge))
            .collect()
    }

    /// Returns the unique mapping for an edge, or `None` when there is no
    /// mapping or when the edge is ambiguous (for example at a corner).
    #[must_use]
    pub fn mapping_for(&self, display_id: &DisplayId, edge: Edge) -> Option<EdgeMapping> {
        let mappings = self.mappings_from(display_id, edge);
        if mappings.len() == 1 {
            mappings.into_iter().next()
        } else if mappings.is_empty() {
            None
        } else {
            // A full-edge mapping wins over a point/corner tie. If two peers
            // share the same maximum segment, the choice remains ambiguous.
            let max = mappings
                .iter()
                .map(EdgeMapping::overlap_length)
                .fold(0.0, f64::max);
            let best: Vec<_> = mappings
                .into_iter()
                .filter(|mapping| {
                    (mapping.overlap_length() - max).abs() <= self.adjacency_tolerance
                })
                .collect();
            (best.len() == 1).then(|| best.into_iter().next().expect("length checked"))
        }
    }

    /// Alias for [`Topology::mapping_for`].
    #[must_use]
    pub fn edge_mapping(&self, display_id: &DisplayId, edge: Edge) -> Option<EdgeMapping> {
        self.mapping_for(display_id, edge)
    }

    /// Returns the unique adjacent mapping, using a display-like argument.
    #[must_use]
    pub fn neighbor(&self, display_id: &DisplayId, edge: Edge) -> Option<EdgeMapping> {
        self.mapping_for(display_id, edge)
    }

    /// Returns all edges which have an adjacent peer.
    #[must_use]
    pub fn adjacent_edges(&self, display_id: &DisplayId) -> Vec<Edge> {
        [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom]
            .into_iter()
            .filter(|edge| !self.mappings_from(display_id, *edge).is_empty())
            .collect()
    }

    fn mapping_between(
        &self,
        source: &DevicePlacement,
        target: &DevicePlacement,
        edge: Edge,
    ) -> Option<EdgeMapping> {
        let tolerance = self.adjacency_tolerance;
        let (touches, target_edge, source_start, source_end, target_start, target_end) = match edge
        {
            Edge::Right => (
                (source.rect.right() - target.rect.x).abs() <= tolerance,
                Edge::Left,
                source.rect.y,
                source.rect.bottom(),
                target.rect.y,
                target.rect.bottom(),
            ),
            Edge::Left => (
                (source.rect.x - target.rect.right()).abs() <= tolerance,
                Edge::Right,
                source.rect.y,
                source.rect.bottom(),
                target.rect.y,
                target.rect.bottom(),
            ),
            Edge::Bottom => (
                (source.rect.bottom() - target.rect.y).abs() <= tolerance,
                Edge::Top,
                source.rect.x,
                source.rect.right(),
                target.rect.x,
                target.rect.right(),
            ),
            Edge::Top => (
                (source.rect.y - target.rect.bottom()).abs() <= tolerance,
                Edge::Bottom,
                source.rect.x,
                source.rect.right(),
                target.rect.x,
                target.rect.right(),
            ),
        };
        if !touches {
            return None;
        }
        let overlap_start = source_start.max(target_start);
        let overlap_end = source_end.min(target_end);
        if overlap_end - overlap_start < self.minimum_overlap {
            return None;
        }
        Some(EdgeMapping {
            from_display: source.display_id.clone(),
            from_edge: edge,
            to_display: target.display_id.clone(),
            to_edge: target_edge,
            overlap_start,
            overlap_end,
            source_span_start: source_start,
            source_span_end: source_end,
            target_span_start: target_start,
            target_span_end: target_end,
            target_overlap_start: overlap_start,
            target_overlap_end: overlap_end,
            target_rect: target.rect,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topology() -> Topology {
        Topology::with_placements(
            "main",
            [
                DevicePlacement::controller("main", Rect::new(0.0, 0.0, 100.0, 100.0)),
                DevicePlacement::controlled("right", Rect::new(100.0, 0.0, 100.0, 100.0)),
                DevicePlacement::controlled("below", Rect::new(0.0, 100.0, 100.0, 100.0)),
            ],
        )
        .expect("valid topology")
    }

    #[test]
    fn maps_left_right_and_top_bottom() {
        let topology = topology();
        let main = DisplayId::from("main");
        let right = topology.mapping_for(&main, Edge::Right).expect("right");
        assert_eq!(right.to_display, DisplayId::from("right"));
        assert_eq!(right.to_edge, Edge::Left);
        let below = topology.mapping_for(&main, Edge::Bottom).expect("bottom");
        assert_eq!(below.to_display, DisplayId::from("below"));
        assert_eq!(below.to_edge, Edge::Top);
    }

    #[test]
    fn corner_only_contact_is_ambiguous_and_not_mapped() {
        let topology = Topology::with_placements(
            "main",
            [
                DevicePlacement::controller("main", Rect::new(0.0, 0.0, 100.0, 100.0)),
                DevicePlacement::controlled("corner", Rect::new(100.0, 100.0, 100.0, 100.0)),
            ],
        )
        .expect("valid topology");
        assert!(
            topology
                .mapping_for(&DisplayId::from("main"), Edge::Right)
                .is_none()
        );
        assert!(
            topology
                .mapping_for(&DisplayId::from("main"), Edge::Bottom)
                .is_none()
        );
    }

    #[test]
    fn mapping_preserves_relative_position() {
        let topology = topology();
        let mapping = topology
            .mapping_for(&DisplayId::from("main"), Edge::Right)
            .expect("mapping");
        assert!((mapping.map_along(25.0) - 25.0).abs() < 1e-6);
        assert!((mapping.map_along(100.0) - 100.0).abs() < 1e-6);
    }
}
