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

    /// 把源屏上的一个点映射到目标屏，并保留越过源边的位移量。
    ///
    /// 与固定内缩不同，越界量会被保留：指针扫进目标屏几个像素，就落在目标屏内
    /// 几个像素，而不是被压回贴着边界的同一个像素。沿边坐标按绝对坐标搬移后再
    /// 钳制到目标矩形，因此两块屏尺寸不同时穿越仍然对齐。
    ///
    /// `source_zone` 把源边按该宽度外扩后再测量越界量。操作系统会把本机指针钳制
    /// 在屏内，本机源在最后一个像素就完成移交，因此需要补偿 `1.0`；远端指针由增量
    /// 积分得到、报告的是真实越界量，取 `0.0`。`target_inset` 让落点再离开进入边
    /// 一个像素，同样只对**本机目标**必要——否则被钳制的本机指针会立刻再次触发。
    ///
    /// 入参与返回值都是拓扑空间坐标。
    #[must_use]
    pub fn map_landing(
        &self,
        source_point: Point,
        source_rect: Rect,
        source_zone: f64,
        target_inset: f64,
    ) -> Point {
        let inward = source_rect.signed_distance_to_edge(source_point, self.from_edge);
        let (along, overflow) = match self.from_edge {
            Edge::Left => (source_point.y, inward - source_zone),
            Edge::Right => (source_point.y, source_zone - inward),
            Edge::Top => (source_point.x, inward - source_zone),
            Edge::Bottom => (source_point.x, source_zone - inward),
        };
        let (mut x, mut y) = match self.from_edge {
            Edge::Left => (self.target_rect.right() + overflow, along),
            Edge::Right => (self.target_rect.x + overflow, along),
            Edge::Top => (along, self.target_rect.bottom() + overflow),
            Edge::Bottom => (along, self.target_rect.y + overflow),
        };
        if target_inset > 0.0 {
            match self.from_edge {
                Edge::Left => x = x.min(self.target_rect.right() - 1.0 - target_inset),
                Edge::Right => x = x.max(self.target_rect.x + target_inset),
                Edge::Top => y = y.min(self.target_rect.bottom() - 1.0 - target_inset),
                Edge::Bottom => y = y.max(self.target_rect.y + target_inset),
            }
        }
        // 内缩在钳制之前，与生产顺序一致：内缩可能把落点推出矩形，由钳制兜底。
        // 用 min/max 而不是 clamp：退化尺寸的屏幕不应让这里 panic。
        x = x
            .max(self.target_rect.x)
            .min(self.target_rect.right() - 1.0);
        y = y
            .max(self.target_rect.y)
            .min(self.target_rect.bottom() - 1.0);
        Point::new(x, y)
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

    /// 对齐生产 `remote_overflow_is_preserved_at_next_display`：指针从中间的远端屏
    /// 再向右越界 7 个像素，落点必须保留这 7 个像素，而不是被压回目标屏的左边缘。
    #[test]
    fn landing_preserves_remote_overflow_beyond_the_target_edge() {
        let topology = Topology::with_placements(
            "a",
            [
                DevicePlacement::controller("a", Rect::new(0.0, 0.0, 100.0, 100.0)),
                DevicePlacement::controlled("b", Rect::new(100.0, 0.0, 100.0, 100.0)),
                DevicePlacement::controlled("c", Rect::new(200.0, 0.0, 100.0, 100.0)),
            ],
        )
        .expect("valid topology");
        let mapping = topology
            .mapping_for(&DisplayId::from("b"), Edge::Right)
            .expect("b to c");
        let source_rect = Rect::new(100.0, 0.0, 100.0, 100.0);
        // 远端指针由增量积分得到，报告的是真实越界量，不需要一点触发区补偿。
        let landing = mapping.map_landing(Point::new(207.0, 50.0), source_rect, 0.0, 0.0);
        assert_eq!(landing, Point::new(207.0, 50.0));
    }

    /// 对齐生产 `instant_crossing...` 的落点 `(0.0, 50.0)`：本机指针被系统钳制，
    /// 在最后一个像素就移交，源边要按一点触发区外扩，否则落点会多算一个像素。
    #[test]
    fn landing_compensates_the_local_one_point_jump_zone() {
        let topology = topology();
        let mapping = topology
            .mapping_for(&DisplayId::from("main"), Edge::Right)
            .expect("main to right");
        let source_rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let landing = mapping.map_landing(Point::new(99.0, 50.0), source_rect, 1.0, 0.0);
        assert_eq!(landing, Point::new(100.0, 50.0));
    }

    /// 目标是本机屏时落点要多内缩一个像素，否则被钳制的本机指针会立刻再次触发
    /// 这条边。对齐生产反向穿越锁死的 `99.0` / `98.0`。
    #[test]
    fn landing_insets_a_local_target_away_from_its_entry_edge() {
        let topology = topology();
        let mapping = topology
            .mapping_for(&DisplayId::from("right"), Edge::Left)
            .expect("right to main");
        let source_rect = Rect::new(100.0, 0.0, 100.0, 100.0);
        let source_point = Point::new(99.0, 50.0);
        assert_eq!(
            mapping.map_landing(source_point, source_rect, 0.0, 0.0),
            Point::new(99.0, 50.0)
        );
        assert_eq!(
            mapping.map_landing(source_point, source_rect, 0.0, 1.0),
            Point::new(98.0, 50.0)
        );
    }

    /// 目标比源矮时，沿边坐标被钳制在目标内，指针不会落到目标屏之外。
    #[test]
    fn landing_clamps_the_along_edge_coordinate_into_a_smaller_target() {
        let topology = Topology::with_placements(
            "main",
            [
                DevicePlacement::controller("main", Rect::new(0.0, 0.0, 100.0, 100.0)),
                DevicePlacement::controlled("short", Rect::new(100.0, 0.0, 100.0, 40.0)),
            ],
        )
        .expect("valid topology");
        let mapping = topology
            .mapping_for(&DisplayId::from("main"), Edge::Right)
            .expect("main to short");
        let source_rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let landing = mapping.map_landing(Point::new(101.0, 80.0), source_rect, 1.0, 0.0);
        assert_eq!(landing, Point::new(102.0, 39.0));
    }

    /// 无论越界多远、目标屏多小，落点都必须留在目标矩形内且不 panic——内缩可能把
    /// 落点推出退化尺寸的目标屏，必须由最后的钳制兜底。
    #[test]
    fn landing_stays_inside_the_target_rect_for_extreme_inputs() {
        let topology = Topology::with_placements(
            "main",
            [
                DevicePlacement::controller("main", Rect::new(0.0, 0.0, 100.0, 100.0)),
                DevicePlacement::controlled("tiny", Rect::new(100.0, 0.0, 1.0, 1.0)),
            ],
        )
        .expect("valid topology");
        let mapping = topology
            .mapping_for(&DisplayId::from("main"), Edge::Right)
            .expect("main to tiny");
        let source_rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        for point in [
            Point::new(101.0, 10.0),
            Point::new(5_000.0, -3_000.0),
            Point::new(-900.0, 99.0),
        ] {
            let landing = mapping.map_landing(point, source_rect, 1.0, 1.0);
            assert_eq!(landing, Point::new(100.0, 0.0), "point {point:?}");
        }
    }
}
