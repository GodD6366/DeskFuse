//! 生产布局与穿越状态机之间的桥接层。
//!
//! 两套建模并不相同，这里是它们唯一的换算入口：
//!
//! - 生产用 [`DisplayKey`]（设备 + 屏幕）描述布局，坐标是「全局绝对点 + 每设备偏移」；
//! - 穿越状态机用拓扑空间的 [`TopologyDisplayId`]（单个字符串）与
//!   [`DevicePlacement`] 矩形，坐标是共享坐标系里的绝对点。
//!
//! 换算只允许在本模块发生。若在调用方各自叠加偏移，就会出现重复平移，而动过
//! 的坐标恰好还能落在某块屏上，属于难以察觉的错误。
//!
//! 注意 [`PointerSample::position`](crate::PointerSample) 必须是拓扑空间坐标，
//! 见 [`topology_point`]。

use crate::sharing::SharingDisplay;
use crate::topology::{
    DevicePlacement, DeviceRole, DisplayId as TopologyDisplayId, Topology, TopologyError,
};
use crate::{Point, Rect};
use deskfuse_core::{DeviceId, DisplayId as CoreDisplayId, DisplayKey};

/// 设备标识与屏幕标识之间的分隔符。
///
/// `DeviceId` 的规范形式是 64 位小写十六进制且不含冒号，屏幕标识由平台提供，
/// 可能包含冒号。因此按**第一个**冒号切分即可无歧义往返。
const KEY_SEPARATOR: char = ':';

/// 把生产布局键编码成拓扑空间的稳定标识。
#[must_use]
pub fn display_id_for(key: &DisplayKey) -> TopologyDisplayId {
    TopologyDisplayId::new(format!(
        "{}{KEY_SEPARATOR}{}",
        key.device_id.to_hex(),
        key.display_id.as_str()
    ))
}

/// 从拓扑标识还原生产布局键。编码非法时返回 `None`。
#[must_use]
pub fn display_key_for(id: &TopologyDisplayId) -> Option<DisplayKey> {
    let (device, display) = id.as_str().split_once(KEY_SEPARATOR)?;
    let device_id = DeviceId::from_hex(device).ok()?;
    Some(DisplayKey {
        device_id,
        display_id: CoreDisplayId::new(display),
    })
}

/// 取出拓扑标识所属的设备。
///
/// 「是否在远端」必须按设备判断，不能拿拓扑标识与本机某一块屏比较：本机可能有
/// 多块屏，指针停在本机副屏上时不该被当成远端。
#[must_use]
pub fn device_of(id: &TopologyDisplayId) -> Option<DeviceId> {
    display_key_for(id).map(|key| key.device_id)
}

/// 由生产布局构造状态机拓扑。
///
/// [`Topology`] 要求**恰好一个** `Controller`，而本机可能有多块屏。因此本机屏中
/// 只有一块被标为控制器：优先 `is_primary`，否则取标识最小的一块以保证结果稳定。
/// 返回的拓扑以该控制器屏为 `local_display`。
///
/// 未就绪的对端仍会进入拓扑：位置保留、几何稳定，可达性由调用方通过
/// [`CrossingConfig`](crate::CrossingConfig) 表达，而不是从这里剔除。
pub fn topology_from_layout(
    local: DeviceId,
    displays: &[SharingDisplay],
) -> Result<Topology, TopologyError> {
    let controller = displays
        .iter()
        .filter(|display| display.key.device_id == local)
        .min_by_key(|display| {
            (
                !display.display.is_primary,
                display_id_for(&display.key).as_str().to_owned(),
            )
        })
        .ok_or(TopologyError::InvalidControllerCount)?;
    let controller_id = display_id_for(&controller.key);

    let placements = displays
        .iter()
        .map(|display| {
            let id = display_id_for(&display.key);
            let role = if id == controller_id {
                DeviceRole::Controller
            } else {
                DeviceRole::Controlled
            };
            DevicePlacement::new(id, display.rect(), role)
        })
        .collect::<Vec<_>>();

    Topology::with_placements(controller_id, placements)
}

/// 屏内坐标 → 拓扑空间坐标。
#[must_use]
pub fn topology_point(display: &SharingDisplay, local: Point) -> Point {
    Point::new(local.x + display.x, local.y + display.y)
}

/// 拓扑空间坐标 → 屏内坐标。
#[must_use]
pub fn local_point(display: &SharingDisplay, topology: Point) -> Point {
    Point::new(topology.x - display.x, topology.y - display.y)
}

/// 与 [`Rect::new`] 一致地描述一块屏在拓扑空间中的范围，供调用方核对换算结果。
#[must_use]
pub fn rect_of(display: &SharingDisplay) -> Rect {
    display.rect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use deskfuse_core::DisplayDescriptor;

    fn device(n: u8) -> DeviceId {
        DeviceId::from_bytes([n; 32])
    }

    fn display(
        dev: u8,
        id: &str,
        os: (f64, f64),
        placed: (f64, f64),
        size: (u32, u32),
        primary: bool,
    ) -> SharingDisplay {
        SharingDisplay {
            key: DisplayKey {
                device_id: device(dev),
                display_id: CoreDisplayId::new(id),
            },
            display: DisplayDescriptor {
                id: CoreDisplayId::new(id),
                frame_x: os.0,
                frame_y: os.1,
                width: size.0,
                height: size.1,
                scale_factor: 2.0,
                rotation_degrees: 0,
                is_primary: primary,
                physical_width_mm: None,
                physical_height_mm: None,
            },
            x: placed.0,
            y: placed.1,
            ready: true,
        }
    }

    #[test]
    fn display_key_round_trips() {
        let key = DisplayKey {
            device_id: device(7),
            display_id: CoreDisplayId::new("main"),
        };
        let id = display_id_for(&key);
        assert_eq!(display_key_for(&id), Some(key.clone()));
        assert_eq!(device_of(&id), Some(device(7)));
    }

    /// 屏幕标识允许包含冒号，编码必须仍能无歧义还原。
    #[test]
    fn display_key_round_trips_when_screen_id_contains_separator() {
        let key = DisplayKey {
            device_id: device(3),
            display_id: CoreDisplayId::new("vendor:model:0"),
        };
        let id = display_id_for(&key);
        assert_eq!(
            id.as_str(),
            format!("{}:vendor:model:0", device(3).to_hex())
        );
        assert_eq!(display_key_for(&id), Some(key));
    }

    #[test]
    fn malformed_ids_are_rejected() {
        assert_eq!(
            display_key_for(&TopologyDisplayId::new("no-separator")),
            None
        );
        assert_eq!(
            display_key_for(&TopologyDisplayId::new("not-hex:screen")),
            None
        );
        assert_eq!(device_of(&TopologyDisplayId::new("zz:screen")), None);
    }

    #[test]
    fn topology_marks_exactly_one_local_screen_as_controller() {
        let displays = vec![
            display(1, "a", (0.0, 0.0), (0.0, 0.0), (100, 100), false),
            display(1, "b", (100.0, 0.0), (100.0, 0.0), (100, 100), true),
            display(2, "c", (-100.0, 0.0), (200.0, 0.0), (100, 100), false),
        ];
        let topology = topology_from_layout(device(1), &displays).expect("valid topology");
        assert_eq!(
            topology
                .placements()
                .filter(|placement| placement.role == DeviceRole::Controller)
                .count(),
            1
        );
        // 本机主屏被选中，即使它不是列表里的第一项。
        assert_eq!(topology.local_display, display_id_for(&displays[1].key));
        // 对端仍是可控屏，位置按布局保留。
        let remote = topology
            .placement(&display_id_for(&displays[2].key))
            .expect("remote placement");
        assert_eq!(remote.role, DeviceRole::Controlled);
        assert_eq!(remote.rect, Rect::new(200.0, 0.0, 100.0, 100.0));
    }

    #[test]
    fn topology_falls_back_to_smallest_id_without_a_primary_screen() {
        let displays = vec![
            display(1, "b", (0.0, 0.0), (0.0, 0.0), (100, 100), false),
            display(1, "a", (100.0, 0.0), (100.0, 0.0), (100, 100), false),
        ];
        let topology = topology_from_layout(device(1), &displays).expect("valid topology");
        assert_eq!(topology.local_display, display_id_for(&displays[1].key));
    }

    #[test]
    fn topology_requires_a_local_display() {
        let displays = vec![display(2, "c", (0.0, 0.0), (0.0, 0.0), (100, 100), true)];
        assert_eq!(
            topology_from_layout(device(1), &displays).unwrap_err(),
            TopologyError::InvalidControllerCount
        );
        assert_eq!(
            topology_from_layout(device(1), &[]).unwrap_err(),
            TopologyError::InvalidControllerCount
        );
    }

    /// 生产布局把对端桌面平移到本机坐标系里；换算必须叠加且只叠加一次该平移。
    #[test]
    fn coordinates_convert_between_display_and_topology_space() {
        // OS 原点 (-100, 0) 的对端屏被放到本机坐标系 (200, 0)：偏移为 300。
        let remote = display(2, "c", (-100.0, 0.0), (200.0, 0.0), (100, 100), false);
        assert_eq!(rect_of(&remote), Rect::new(200.0, 0.0, 100.0, 100.0));
        // 屏内左上角对应拓扑空间里的 (200, 0)。
        assert_eq!(
            topology_point(&remote, Point::new(0.0, 0.0)),
            Point::new(200.0, 0.0)
        );
        assert_eq!(
            local_point(&remote, Point::new(200.0, 0.0)),
            Point::new(0.0, 0.0)
        );
        // 往返一致。
        let local = Point::new(37.0, 42.0);
        assert_eq!(local_point(&remote, topology_point(&remote, local)), local);
    }

    /// 拓扑空间坐标必须落在该屏的矩形内，否则状态机的边距判定没有意义。
    #[test]
    fn topology_point_lands_inside_the_placement_rect() {
        let remote = display(2, "c", (0.0, 0.0), (200.0, 0.0), (100, 100), false);
        let inside = topology_point(&remote, Point::new(50.0, 50.0));
        let rect = rect_of(&remote);
        assert!(inside.x >= rect.x && inside.x <= rect.x + rect.width);
        assert!(inside.y >= rect.y && inside.y <= rect.y + rect.height);
    }
}
