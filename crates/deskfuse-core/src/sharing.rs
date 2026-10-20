//! Explicitly authorized input-sharing types, separate from legacy input RPCs.
use crate::{DeviceId, NetworkError, NetworkErrorCode, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    net::SocketAddr,
};
use uuid::Uuid;

macro_rules! session_id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
        )]
        pub struct $name(pub Uuid);
        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}
session_id!(InputSessionId);
session_id!(InputActivationId);
session_id!(TransportSessionId);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum LinkScope {
    Lan,
    Overlay,
    Public,
    Unknown,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum InterfaceKind {
    WiFi,
    Ethernet,
    Thunderbolt,
    Vpn,
    Loopback,
    Other,
}
impl InterfaceKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Thunderbolt => "雷电",
            Self::Ethernet => "有线",
            Self::WiFi => "Wi-Fi",
            Self::Vpn => "VPN",
            Self::Loopback => "本机",
            Self::Other => "",
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConnectionContext {
    pub session_id: TransportSessionId,
    pub local_addr: SocketAddr,
    pub remote_addr: SocketAddr,
    pub interface_name: Option<String>,
    pub interface_kind: InterfaceKind,
    pub scope: LinkScope,
}
impl ConnectionContext {
    pub fn input_eligible(&self) -> bool {
        self.scope == LinkScope::Lan
            && matches!(
                self.interface_kind,
                InterfaceKind::WiFi | InterfaceKind::Ethernet | InterfaceKind::Thunderbolt
            )
            && !self.local_addr.ip().is_loopback()
            && !self.remote_addr.ip().is_loopback()
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct DisplayId(pub String);
impl DisplayId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl From<&str> for DisplayId {
    fn from(id: &str) -> Self {
        Self(id.into())
    }
}
impl From<String> for DisplayId {
    fn from(id: String) -> Self {
        Self(id)
    }
}
impl From<u64> for DisplayId {
    fn from(id: u64) -> Self {
        Self(id.to_string())
    }
}
impl fmt::Display for DisplayId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct DisplayKey {
    pub device_id: DeviceId,
    pub display_id: DisplayId,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplayDescriptor {
    pub id: DisplayId,
    pub frame_x: f64,
    pub frame_y: f64,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
    pub rotation_degrees: u16,
    pub is_primary: bool,
    /// Optional physical panel width reported by the native display adapter.
    /// Logical coordinates and scale remain authoritative for input injection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_width_mm: Option<f64>,
    /// Optional physical panel height reported by the native display adapter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_height_mm: Option<f64>,
}
impl DisplayDescriptor {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x.is_finite()
            && y.is_finite()
            && x >= 0.0
            && y >= 0.0
            && x < f64::from(self.width)
            && y < f64::from(self.height)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplayTopology {
    pub generation: u64,
    pub displays: Vec<DisplayDescriptor>,
}
impl DisplayTopology {
    pub fn validate(&self) -> Result<()> {
        let mut ids = BTreeSet::new();
        if self.displays.is_empty()
            || self.displays.len() > 32
            || self.displays.iter().any(|d| {
                d.id.0.is_empty()
                    || d.id.0.len() > 128
                    || !ids.insert(&d.id)
                    || !d.frame_x.is_finite()
                    || !d.frame_y.is_finite()
                    || d.width == 0
                    || d.height == 0
                    || d.width > 65536
                    || d.height > 65536
                    || !d.scale_factor.is_finite()
                    || !(0.25..=8.0).contains(&d.scale_factor)
                    || ![0, 90, 180, 270].contains(&d.rotation_degrees)
                    || d.physical_width_mm
                        .is_some_and(|v| !v.is_finite() || !(1.0..=10_000.0).contains(&v))
                    || d.physical_height_mm
                        .is_some_and(|v| !v.is_finite() || !(1.0..=10_000.0).contains(&v))
            })
        {
            return Err(invalid("invalid display topology"));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum EdgeSwitchMode {
    #[default]
    Instant,
    Dwell250Ms,
    DoubleTap500Ms,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum InputShareRole {
    #[default]
    Disabled,
    Controller,
    Controlled,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplayPlacementSetting {
    pub display: DisplayKey,
    pub x: f64,
    pub y: f64,
}
/// Side of a device relative to its neighboring desktop.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DisplayLayoutSide {
    Left,
    Right,
    Above,
    Below,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplayLayoutAttachment {
    pub neighbor: DeviceId,
    pub side: DisplayLayoutSide,
}
/// A device's placement in a particular controller/receiver screen combination.
/// Connection readiness and topology generation are deliberately not identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplayLayoutProfile {
    pub device_id: DeviceId,
    pub local_topology: String,
    pub device_topology: String,
    /// Remote positions are relative to the controller desktop's minimum origin.
    pub placements: Vec<DisplayPlacementSetting>,
    pub attachment: Option<DisplayLayoutAttachment>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputSharingSettings {
    /// Local master gate, independent from the saved controller role.
    pub feature_enabled: bool,
    pub shortcuts: InputShortcutBindings,
    pub disabled_corners: u8,
    pub corner_size: u16,
    pub enabled: bool,
    pub role: InputShareRole,
    pub authorized_controller: Option<DeviceId>,
    pub participants: Vec<DeviceId>,
    pub switch_mode: EdgeSwitchMode,
    pub placements: Vec<DisplayPlacementSetting>,
    pub layout_profiles: Vec<DisplayLayoutProfile>,
}
impl Default for InputSharingSettings {
    fn default() -> Self {
        Self {
            feature_enabled: true,
            shortcuts: InputShortcutBindings::default(),
            disabled_corners: 0,
            corner_size: 16,
            enabled: false,
            role: InputShareRole::Disabled,
            authorized_controller: None,
            participants: vec![],
            switch_mode: EdgeSwitchMode::Instant,
            placements: vec![],
            layout_profiles: vec![],
        }
    }
}
impl InputSharingSettings {
    pub fn validate(&self) -> Result<()> {
        let shortcuts = [
            &self.shortcuts.left,
            &self.shortcuts.right,
            &self.shortcuts.up,
            &self.shortcuts.down,
            &self.shortcuts.lock,
        ];
        if shortcuts.iter().any(|s| {
            s.modifiers == 0
                || s.modifiers & !0x1E0000 != 0
                || (s.key_code == 53 && s.modifiers == 0x1C0000)
        }) || shortcuts
            .iter()
            .map(|s| (s.key_code, s.modifiers))
            .collect::<BTreeSet<_>>()
            .len()
            != 5
        {
            return Err(invalid(
                "shortcut chords must be distinct, use valid modifiers, and preserve emergency reclaim",
            ));
        }

        let emergency = &self.shortcuts.emergency;
        if emergency.key_code > 127
            || emergency.modifiers & 0x1C0000 == 0
            || emergency.modifiers & !0x1E0000 != 0
            || (emergency.key_code == 2 && emergency.modifiers == 0x120000)
            || shortcuts.contains(&emergency)
        {
            return Err(invalid(
                "emergency shortcut must be a distinct valid modifier chord",
            ));
        }

        if self.disabled_corners > 15
            || self.corner_size > 256
            || [
                &self.shortcuts.left,
                &self.shortcuts.right,
                &self.shortcuts.up,
                &self.shortcuts.down,
                &self.shortcuts.lock,
            ]
            .iter()
            .any(|s| s.key_code > 255)
            || self.participants.len() > 32
            || self.layout_profiles.len() > 128
            || self.layout_profiles.iter().any(|profile| {
                profile.local_topology.is_empty()
                    || profile.local_topology.len() > 16384
                    || profile.device_topology.is_empty()
                    || profile.device_topology.len() > 16384
                    || profile.placements.is_empty()
                    || profile.placements.len() > 32
                    || profile.placements.iter().any(|p| {
                        p.display.device_id != profile.device_id
                            || !p.x.is_finite()
                            || !p.y.is_finite()
                    })
                    || profile
                        .placements
                        .iter()
                        .map(|p| &p.display)
                        .collect::<BTreeSet<_>>()
                        .len()
                        != profile.placements.len()
                    || profile
                        .attachment
                        .as_ref()
                        .is_some_and(|a| a.neighbor == profile.device_id)
            })
            || self
                .layout_profiles
                .iter()
                .map(|p| (p.device_id, &p.local_topology, &p.device_topology))
                .collect::<BTreeSet<_>>()
                .len()
                != self.layout_profiles.len()
            || self.placements.len() > 128
            || self.participants.iter().collect::<BTreeSet<_>>().len() != self.participants.len()
            || self
                .placements
                .iter()
                .map(|p| &p.display)
                .collect::<BTreeSet<_>>()
                .len()
                != self.placements.len()
            || self
                .placements
                .iter()
                .any(|p| !p.x.is_finite() || !p.y.is_finite())
            || (self.enabled && self.role == InputShareRole::Disabled)
            || (self.enabled
                && self.role == InputShareRole::Controlled
                && self.authorized_controller.is_none())
        {
            return Err(invalid("invalid input sharing settings"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SharedInputEvent {
    Motion {
        x: f64,
        y: f64,
    },
    Button {
        button: u8,
        pressed: bool,
    },
    Key {
        code: u16,
        pressed: bool,
        modifiers: u64,
        repeat: bool,
    },
    Modifiers {
        modifiers: u64,
    },
    Scroll {
        dx: f64,
        dy: f64,
    },
    /// 设备计数相对位移。+x 向右，+y 向下。只在进程内从接收走到注入，不进入可靠 Hybrid JSON。
    RelativeMotion {
        dx: f64,
        dy: f64,
    },
}
impl SharedInputEvent {
    pub fn validate(&self) -> Result<()> {
        let valid = match self {
            Self::Motion { x, y } => {
                x.is_finite()
                    && y.is_finite()
                    && *x >= 0.0
                    && *y >= 0.0
                    && *x <= 65536.0
                    && *y <= 65536.0
            }
            Self::Scroll { dx, dy } => {
                dx.is_finite() && dy.is_finite() && dx.abs() <= 65536.0 && dy.abs() <= 65536.0
            }
            Self::Button { button, .. } => *button <= 31,
            Self::Key { code, .. } => *code <= 255,
            Self::Modifiers { .. } => true,
            Self::RelativeMotion { dx, dy } => {
                crate::relative_delta_ok(*dx) && crate::relative_delta_ok(*dy)
            }
        };
        if valid {
            Ok(())
        } else {
            Err(invalid("invalid shared input event"))
        }
    }
}
/// Capability token; never expose the bytes through diagnostics.
#[derive(Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub struct InputToken(pub [u8; 32]);
impl InputToken {
    pub fn new() -> Self {
        let mut b = [0; 32];
        b[..16].copy_from_slice(Uuid::new_v4().as_bytes());
        b[16..].copy_from_slice(Uuid::new_v4().as_bytes());
        Self(b)
    }
}
impl Default for InputToken {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Debug for InputToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InputToken([redacted])")
    }
}
/// Absolute display-local position shared by unreliable samples and reliable anchors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionPosition {
    pub target_display: DisplayId,
    pub topology_generation: u64,
    /// Strictly increasing per-session position sample number, independent of control.
    pub motion_seq: u64,
    pub x: f64,
    pub y: f64,
}
impl MotionPosition {
    pub fn validate(&self) -> Result<()> {
        validate_display(&self.target_display)?;
        if self.motion_seq == 0 {
            return Err(invalid("zero motion sequence"));
        }
        SharedInputEvent::Motion {
            x: self.x,
            y: self.y,
        }
        .validate()
    }
}
/// Unreliable latest-position update. Authorization remains bound to the QUIC peer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionDatagram {
    pub session_id: InputSessionId,
    pub token: InputToken,
    /// Reliable Enter/Transition sequence whose input state this sample depends on.
    pub state_epoch: u64,
    pub position: MotionPosition,
}
impl MotionDatagram {
    pub fn validate(&self) -> Result<()> {
        if self.session_id.0.is_nil() || self.token.0 == [0; 32] || self.state_epoch == 0 {
            return Err(invalid("invalid motion identity or state epoch"));
        }
        self.position.validate()
    }
}
/// 一次会话选定的指针语义。续租不能改它。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionPointerMode {
    /// 绝对逻辑坐标。datagram 可用时对外仍是 hybrid。
    #[default]
    Absolute,
    /// 设备计数相对位移。
    Relative,
    /// 只有可靠输入。
    Reliable,
}
impl MotionPointerMode {
    /// serde 跳过默认绝对模式，让旧 JSON 往返保持不变。
    pub fn is_absolute(mode: &Self) -> bool {
        matches!(mode, Self::Absolute)
    }
}
/// 不可靠相对位移。授权仍绑定 QUIC 对端和会话令牌。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RelativeMotionDatagram {
    pub session_id: InputSessionId,
    pub token: InputToken,
    /// 可靠 Enter/Transition 建立的输入状态世代。
    pub state_epoch: u64,
    /// 与绝对坐标共用的会话内单调序号，从 1 开始。
    pub motion_seq: u64,
    /// 向右为正的设备计数。
    pub dx: f64,
    /// 向下为正的设备计数。
    pub dy: f64,
}
impl RelativeMotionDatagram {
    /// 拒绝空身份、零序号、非有限值和超过 32768 的增量。
    pub fn validate(&self) -> Result<()> {
        if self.session_id.0.is_nil()
            || self.token.0 == [0; 32]
            || self.state_epoch == 0
            || self.motion_seq == 0
        {
            return Err(invalid("invalid relative motion identity"));
        }
        if !crate::relative_delta_ok(self.dx) || !crate::relative_delta_ok(self.dy) {
            return Err(invalid("invalid relative motion delta"));
        }
        Ok(())
    }
}
/// Reliable barriers and convergence snapshots for a negotiated hybrid session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum HybridInputUpdate {
    /// Establish position and activate the enclosing reliable sequence as state epoch.
    Enter { position: MotionPosition },
    /// Position under the old input state, followed by ordered state-changing events.
    /// Motion is forbidden here: position changes require a numbered anchor or sample.
    /// The enclosing reliable sequence becomes the new state epoch after these events.
    Transition {
        position: MotionPosition,
        events: Vec<SharedInputEvent>,
    },
    /// Latest final position; does not change the current input-state epoch.
    Snapshot {
        state_epoch: u64,
        position: MotionPosition,
    },
}
impl HybridInputUpdate {
    pub fn position(&self) -> &MotionPosition {
        match self {
            Self::Enter { position }
            | Self::Transition { position, .. }
            | Self::Snapshot { position, .. } => position,
        }
    }
    pub fn validate(&self) -> Result<()> {
        self.position().validate()?;
        match self {
            Self::Transition { events, .. } => {
                if events.is_empty() || events.len() > 254 {
                    return Err(invalid("invalid hybrid transition bounds"));
                }
                for event in events {
                    if matches!(
                        event,
                        SharedInputEvent::Motion { .. } | SharedInputEvent::RelativeMotion { .. }
                    ) {
                        return Err(invalid("motion is not allowed inside a hybrid transition"));
                    }
                    event.validate()?;
                }
            }
            Self::Snapshot { state_epoch: 0, .. } => {
                return Err(invalid("zero snapshot state epoch"));
            }
            _ => {}
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InputShareMessage {
    ForgetControllerEnvironment {
        ssid: String,
    },
    ControllerPresence {
        controller: bool,
        ssid: Option<String>,
        preferred: Option<DeviceId>,
    },
    ControllerSelection {
        controller: DeviceId,
        ssid: Option<String>,
        remember: bool,
    },
    Activate {
        activation_id: InputActivationId,
        intent_id: InputActivationId,
        ttl_ms: u32,
        sequence: u64,
        #[serde(default)]
        force: bool,
    },
    Activated {
        activation_id: InputActivationId,
        ttl_ms: u32,
        sequence: u64,
        status: InputActivationStatus,
        reason: Option<String>,
    },
    Deactivate {
        activation_id: InputActivationId,
        sequence: u64,
    },
    RequestHybrid {
        session_id: InputSessionId,
        target_display: DisplayId,
        /// 控制端原始相对采集已就绪。旧请求缺省为否，因此仍走绝对坐标。
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        relative_ready: bool,
    },
    GrantHybrid {
        session_id: InputSessionId,
        token: InputToken,
        ttl_ms: u32,
        motion_enabled: bool,
        /// 本租约固定的指针模式。缺省绝对坐标，续租必须原样回显。
        #[serde(default, skip_serializing_if = "MotionPointerMode::is_absolute")]
        pointer_mode: MotionPointerMode,
        /// 没有选用相对模式时的原因。不记录坐标。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pointer_fallback: Option<String>,
    },
    Hybrid {
        session_id: InputSessionId,
        token: InputToken,
        sequence: u64,
        update: HybridInputUpdate,
    },
    Request {
        session_id: InputSessionId,
        target_display: DisplayId,
    },
    Grant {
        session_id: InputSessionId,
        token: InputToken,
        ttl_ms: u32,
    },
    Denied {
        session_id: InputSessionId,
        reason: String,
    },
    Enter {
        session_id: InputSessionId,
        token: InputToken,
        target_display: DisplayId,
        x: f64,
        y: f64,
        sequence: u64,
    },
    Frame {
        session_id: InputSessionId,
        token: InputToken,
        target_display: DisplayId,
        sequence: u64,
        topology_generation: u64,
        events: Vec<SharedInputEvent>,
    },
    Renew {
        session_id: InputSessionId,
        token: InputToken,
        sequence: u64,
    },
    Leave {
        session_id: InputSessionId,
        token: InputToken,
        sequence: u64,
    },
    Release {
        session_id: InputSessionId,
        token: InputToken,
        sequence: u64,
    },
    DisplayTopology(DisplayTopology),
    /// Small JPEG preview, sent independently of geometry so artwork never
    /// changes an input lease or the layout generation.
    DisplayWallpaper {
        display_id: DisplayId,
        #[serde(with = "wallpaper_base64")]
        jpeg: Vec<u8>,
    },
}
pub const MAX_DISPLAY_WALLPAPER_BYTES: usize = 64 * 1024;
pub const MAX_DISPLAY_WALLPAPER_BASE64_BYTES: usize = MAX_DISPLAY_WALLPAPER_BYTES.div_ceil(3) * 4;

mod wallpaper_base64 {
    use super::MAX_DISPLAY_WALLPAPER_BASE64_BYTES;
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        // Reject oversized untrusted artwork before allocating decoded bytes.
        if encoded.len() > MAX_DISPLAY_WALLPAPER_BASE64_BYTES {
            return Err(D::Error::custom("wallpaper preview too large"));
        }
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(D::Error::custom)
    }
}
impl InputShareMessage {
    pub fn session_id(&self) -> Option<InputSessionId> {
        match self {
            Self::RequestHybrid { session_id, .. }
            | Self::GrantHybrid { session_id, .. }
            | Self::Hybrid { session_id, .. }
            | Self::Request { session_id, .. }
            | Self::Grant { session_id, .. }
            | Self::Denied { session_id, .. }
            | Self::Enter { session_id, .. }
            | Self::Frame { session_id, .. }
            | Self::Renew { session_id, .. }
            | Self::Leave { session_id, .. }
            | Self::Release { session_id, .. } => Some(*session_id),
            Self::ForgetControllerEnvironment { .. }
            | Self::ControllerPresence { .. }
            | Self::ControllerSelection { .. }
            | Self::DisplayTopology(_)
            | Self::DisplayWallpaper { .. }
            | Self::Activate { .. }
            | Self::Activated { .. }
            | Self::Deactivate { .. } => None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if let Self::ForgetControllerEnvironment { ssid } = self
            && (ssid.is_empty() || ssid.len() > 32)
        {
            return Err(invalid("invalid Wi-Fi environment"));
        }
        match self {
            Self::ControllerPresence { ssid, .. } | Self::ControllerSelection { ssid, .. }
                if ssid.as_ref().is_some_and(|s| s.is_empty() || s.len() > 32) =>
            {
                return Err(invalid("invalid Wi-Fi environment"));
            }
            _ => {}
        }
        match self {
            Self::RequestHybrid { session_id, .. } if session_id.0.is_nil() => {
                return Err(invalid("invalid hybrid request identity"));
            }
            Self::GrantHybrid {
                session_id,
                token,
                pointer_fallback,
                ..
            } if session_id.0.is_nil()
                || token.0 == [0; 32]
                || pointer_fallback
                    .as_ref()
                    .is_some_and(|reason| reason.len() > 256) =>
            {
                return Err(invalid("invalid hybrid grant identity"));
            }
            _ => {}
        }
        match self {
            Self::Activate {
                ttl_ms, sequence, ..
            }
            | Self::Activated {
                ttl_ms, sequence, ..
            } if *ttl_ms != 1500 || *sequence == 0 => {
                return Err(invalid("invalid activation lease or sequence"));
            }
            Self::Activated {
                reason: Some(reason),
                ..
            } if reason.len() > 256 => {
                return Err(invalid("activation reason too long"));
            }
            Self::Deactivate { sequence: 0, .. } => {
                return Err(invalid("zero activation sequence"));
            }
            Self::DisplayTopology(t) => t.validate()?,
            Self::DisplayWallpaper { display_id, jpeg } => {
                if display_id.0.is_empty()
                    || display_id.0.len() > 128
                    || jpeg.is_empty()
                    || jpeg.len() > MAX_DISPLAY_WALLPAPER_BYTES
                    || !jpeg.starts_with(&[0xff, 0xd8, 0xff])
                {
                    return Err(invalid("invalid display wallpaper preview"));
                }
            }
            Self::Grant { ttl_ms, .. } | Self::GrantHybrid { ttl_ms, .. } if *ttl_ms != 1500 => {
                return Err(invalid("invalid input lease duration"));
            }
            Self::Denied { reason, .. } if reason.len() > 256 => {
                return Err(invalid("input denial too long"));
            }
            Self::Enter {
                x,
                y,
                target_display,
                sequence,
                ..
            } => {
                validate_display(target_display)?;
                SharedInputEvent::Motion { x: *x, y: *y }.validate()?;
                if *sequence == 0 {
                    return Err(invalid("zero input sequence"));
                }
            }
            Self::Request { target_display, .. } | Self::RequestHybrid { target_display, .. } => {
                validate_display(target_display)?
            }
            Self::Hybrid {
                session_id,
                token,
                sequence,
                update,
            } => {
                if session_id.0.is_nil() || token.0 == [0; 32] || *sequence == 0 {
                    return Err(invalid("invalid hybrid identity or sequence"));
                }
                update.validate()?;
                if matches!(update, HybridInputUpdate::Snapshot { state_epoch, .. } if state_epoch > sequence)
                {
                    return Err(invalid("snapshot epoch exceeds reliable sequence"));
                }
            }
            Self::Frame {
                target_display,
                sequence,
                events,
                ..
            } => {
                validate_display(target_display)?;
                if *sequence == 0 || events.is_empty() || events.len() > 256 {
                    return Err(invalid("invalid input frame bounds"));
                }
                for event in events {
                    event.validate()?;
                }
            }
            Self::Renew { sequence, .. }
            | Self::Leave { sequence, .. }
            | Self::Release { sequence, .. }
                if *sequence == 0 =>
            {
                return Err(invalid("zero input sequence"));
            }
            _ => {}
        }
        Ok(())
    }
}
fn validate_display(id: &DisplayId) -> Result<()> {
    if id.0.is_empty() || id.0.len() > 128 {
        Err(invalid("invalid display ID"))
    } else {
        Ok(())
    }
}
fn invalid(message: &str) -> NetworkError {
    NetworkError::new(NetworkErrorCode::InvalidPayload, message)
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InputPeerStatus {
    /// Negotiated lease mode: hybrid or reliable-only; absent without an input lease.
    #[serde(default)]
    pub motion_mode: Option<String>,
    /// Fixed reason for reliable-only selection in this lease, not a readiness failure.
    #[serde(default)]
    pub motion_fallback_reason: Option<String>,
    pub lease_revision: u64,
    pub peer_id: DeviceId,
    pub context: Option<ConnectionContext>,
    pub ready: bool,
    pub reason: Option<String>,
    pub control_session: Option<InputSessionId>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum InputActivationStatus {
    Preparing,
    Ready,
    WaitingAccessibility,
    WaitingInputMonitoring,
    LockedSession,
    BusyLocalController,
    BusyOtherController,
    PausedByUser,
    Unavailable,
}
impl InputActivationStatus {
    pub fn is_native_status(self) -> bool {
        matches!(
            self,
            Self::Preparing
                | Self::Ready
                | Self::WaitingAccessibility
                | Self::WaitingInputMonitoring
                | Self::LockedSession
                | Self::Unavailable
        )
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManagedInputControl {
    pub activation_id: InputActivationId,
    pub controller: DeviceId,
    pub context: ConnectionContext,
    pub status: InputActivationStatus,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InputActivationState {
    pub activation_id: InputActivationId,
    pub status: InputActivationStatus,
    pub reason: Option<String>,
}
/// Durable local enable-cycle epochs and emergency pauses; never sent as settings.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct InputActivationPolicy {
    #[serde(default)]
    pub environment_rules: BTreeMap<String, DeviceId>,
    pub controller_intents: BTreeMap<DeviceId, InputActivationId>,
    /// The most recent local emergency pause for each controller device.
    pub paused_intents: BTreeSet<(DeviceId, InputActivationId)>,
}
impl InputActivationPolicy {
    pub fn validate(&self) -> Result<()> {
        if self.environment_rules.len() > 64
            || self
                .environment_rules
                .keys()
                .any(|s| s.is_empty() || s.len() > 32)
            || self.controller_intents.len() > 32
            || self.paused_intents.len() > 1024
            || self.controller_intents.values().any(|id| id.0.is_nil())
            || self.paused_intents.iter().any(|(_, id)| id.0.is_nil())
            || self
                .paused_intents
                .iter()
                .map(|(peer, _)| peer)
                .collect::<BTreeSet<_>>()
                .len()
                != self.paused_intents.len()
        {
            return Err(invalid("invalid activation policy"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ControllerEnvironmentSnapshot {
    pub ssid: Option<String>,
    pub rules: BTreeMap<String, DeviceId>,
    pub candidates: Vec<DeviceId>,
    pub selected: Option<DeviceId>,
    pub conflict: bool,
    pub local_suspended: bool,
    pub message: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InputSharingSnapshot {
    pub environment: ControllerEnvironmentSnapshot,
    pub managed_control: Option<ManagedInputControl>,
    pub activations: BTreeMap<DeviceId, InputActivationState>,
    pub scenes: Vec<InputSharingScene>,
    pub settings: InputSharingSettings,
    pub local_topology: Option<DisplayTopology>,
    pub remote_topologies: BTreeMap<DeviceId, DisplayTopology>,
    #[serde(default, with = "remote_wallpapers_base64")]
    pub remote_wallpapers: BTreeMap<DeviceId, BTreeMap<DisplayId, Vec<u8>>>,
    pub links: Vec<InputPeerStatus>,
}
/// Keep image previews below the input-service status IPC frame limit, even
/// when a trusted peer advertises the maximum 32 displays.
pub const MAX_REMOTE_WALLPAPER_PREVIEWS: usize = 16;

mod remote_wallpapers_base64 {
    use super::{
        DeviceId, DisplayId, MAX_DISPLAY_WALLPAPER_BASE64_BYTES, MAX_REMOTE_WALLPAPER_PREVIEWS,
    };
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};
    use std::collections::BTreeMap;

    type Previews = BTreeMap<DeviceId, BTreeMap<DisplayId, Vec<u8>>>;
    type Encoded = BTreeMap<DeviceId, BTreeMap<DisplayId, String>>;

    pub fn serialize<S: Serializer>(previews: &Previews, serializer: S) -> Result<S::Ok, S::Error> {
        let encoded: Encoded = previews
            .iter()
            .map(|(peer, displays)| {
                (
                    *peer,
                    displays
                        .iter()
                        .map(|(id, jpeg)| {
                            (
                                id.clone(),
                                base64::engine::general_purpose::STANDARD.encode(jpeg),
                            )
                        })
                        .collect(),
                )
            })
            .collect();
        encoded.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Previews, D::Error> {
        let encoded = Encoded::deserialize(deserializer)?;
        if encoded.values().map(BTreeMap::len).sum::<usize>() > MAX_REMOTE_WALLPAPER_PREVIEWS {
            return Err(D::Error::custom("too many wallpaper previews"));
        }
        encoded
            .into_iter()
            .map(|(peer, displays)| {
                let decoded: Result<BTreeMap<_, _>, D::Error> = displays
                    .into_iter()
                    .map(|(id, jpeg)| {
                        if jpeg.len() > MAX_DISPLAY_WALLPAPER_BASE64_BYTES {
                            return Err(D::Error::custom("wallpaper preview too large"));
                        }
                        let bytes = base64::engine::general_purpose::STANDARD
                            .decode(jpeg)
                            .map_err(D::Error::custom)?;
                        Ok((id, bytes))
                    })
                    .collect();
                decoded.map(|displays| (peer, displays))
            })
            .collect()
    }
}

impl InputSharingSnapshot {
    /// Receiving keyboard/mouse control must never enable a nested remote-app
    /// controller. A user-paused incoming activation still owns the receiver
    /// role; saved local settings alone cannot override it.
    pub fn can_operate_remote_applications(&self) -> bool {
        self.managed_control.is_none() && (!self.settings.enabled || self.can_configure_layout())
    }

    /// Layout authoring belongs only to the selected local controller, never a receiver.
    pub fn can_configure_layout(&self) -> bool {
        self.managed_control.is_none()
            && !self.environment.local_suspended
            && self.settings.role == InputShareRole::Controller
    }

    pub fn effective_settings(&self) -> InputSharingSettings {
        let mut settings = self.settings.clone();
        if !settings.feature_enabled {
            settings.enabled = false;
            settings.role = InputShareRole::Disabled;
            settings.authorized_controller = None;
        } else if let Some(managed) = &self.managed_control {
            settings.enabled = managed.status != InputActivationStatus::PausedByUser;
            settings.role = if settings.enabled {
                InputShareRole::Controlled
            } else {
                InputShareRole::Disabled
            };
            settings.authorized_controller = settings.enabled.then_some(managed.controller);
        } else if self.environment.local_suspended || settings.role != InputShareRole::Controller {
            settings.enabled = false;
            settings.role = InputShareRole::Disabled;
            settings.authorized_controller = None;
        }
        settings
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InputShortcut {
    pub key_code: u16,
    pub modifiers: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InputShortcutBindings {
    pub left: InputShortcut,
    pub right: InputShortcut,
    pub up: InputShortcut,
    pub down: InputShortcut,
    pub lock: InputShortcut,
    #[serde(default = "default_emergency_shortcut")]
    pub emergency: InputShortcut,
}
fn default_emergency_shortcut() -> InputShortcut {
    InputShortcut {
        key_code: 53,
        modifiers: 0x1C0000,
    }
}
impl Default for InputShortcutBindings {
    fn default() -> Self {
        let chord = |key_code| InputShortcut {
            key_code,
            modifiers: 0x1C0000,
        };
        Self {
            left: chord(123),
            right: chord(124),
            up: chord(126),
            down: chord(125),
            lock: chord(37),
            emergency: default_emergency_shortcut(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hybrid_position() -> MotionPosition {
        MotionPosition {
            target_display: DisplayId::new("main"),
            topology_generation: 2,
            motion_seq: 1,
            x: 1.,
            y: 2.,
        }
    }

    #[test]
    fn wallpaper_preview_is_bounded_and_uses_compact_wire_encoding() {
        let message = InputShareMessage::DisplayWallpaper {
            display_id: DisplayId::new("screen"),
            jpeg: [
                vec![0xff, 0xd8, 0xff],
                vec![42; MAX_DISPLAY_WALLPAPER_BYTES - 3],
            ]
            .concat(),
        };
        message.validate().unwrap();
        let encoded = serde_json::to_vec(&message).unwrap();
        assert!(encoded.len() < 100_000);
        assert_eq!(
            serde_json::from_slice::<InputShareMessage>(&encoded).unwrap(),
            message
        );
        let too_large = InputShareMessage::DisplayWallpaper {
            display_id: DisplayId::new("screen"),
            jpeg: [
                vec![0xff, 0xd8, 0xff],
                vec![42; MAX_DISPLAY_WALLPAPER_BYTES - 2],
            ]
            .concat(),
        };
        assert!(too_large.validate().is_err());
    }

    #[test]
    fn hybrid_messages_validate_serialize_and_expose_session() {
        let session_id = InputSessionId::new();
        let token = InputToken::new();
        let position = hybrid_position();
        let messages = vec![
            InputShareMessage::RequestHybrid {
                session_id,
                target_display: DisplayId::new("main"),
                relative_ready: false,
            },
            InputShareMessage::GrantHybrid {
                session_id,
                token,
                ttl_ms: 1500,
                motion_enabled: true,
                pointer_mode: MotionPointerMode::Absolute,
                pointer_fallback: None,
            },
            InputShareMessage::GrantHybrid {
                session_id,
                token,
                ttl_ms: 1500,
                motion_enabled: false,
                pointer_mode: MotionPointerMode::Absolute,
                pointer_fallback: None,
            },
            InputShareMessage::Hybrid {
                session_id,
                token,
                sequence: 2,
                update: HybridInputUpdate::Enter {
                    position: position.clone(),
                },
            },
            InputShareMessage::Hybrid {
                session_id,
                token,
                sequence: 3,
                update: HybridInputUpdate::Transition {
                    position: position.clone(),
                    events: vec![SharedInputEvent::Button {
                        button: 0,
                        pressed: true,
                    }],
                },
            },
            InputShareMessage::Hybrid {
                session_id,
                token,
                sequence: 5,
                update: HybridInputUpdate::Snapshot {
                    state_epoch: 3,
                    position,
                },
            },
        ];
        for message in messages {
            message.validate().unwrap();
            assert_eq!(message.session_id(), Some(session_id));
            let encoded = serde_json::to_vec(&message).unwrap();
            assert_eq!(
                serde_json::from_slice::<InputShareMessage>(&encoded).unwrap(),
                message
            );
        }
        assert_eq!(crate::Capabilities::INPUT_MOTION_DATAGRAM_V1.bits(), 1 << 9);
    }

    #[test]
    fn hybrid_bounds_and_identity_are_checked_before_use() {
        let mut position = hybrid_position();
        position.motion_seq = 0;
        assert!(position.validate().is_err());
        position.motion_seq = 1;
        position.target_display = DisplayId::new("x".repeat(129));
        assert!(position.validate().is_err());
        let position = hybrid_position();
        for count in [0, 255, 256, 257] {
            let update = HybridInputUpdate::Transition {
                position: position.clone(),
                events: vec![SharedInputEvent::Modifiers { modifiers: 0 }; count],
            };
            assert!(update.validate().is_err());
        }
        let snapshot = InputShareMessage::Hybrid {
            session_id: InputSessionId::new(),
            token: InputToken::new(),
            sequence: 3,
            update: HybridInputUpdate::Snapshot {
                state_epoch: 4,
                position,
            },
        };
        assert!(snapshot.validate().is_err());
        assert!(
            InputShareMessage::RequestHybrid {
                session_id: InputSessionId(Uuid::nil()),
                target_display: DisplayId::new("main"),
                relative_ready: false,
            }
            .validate()
            .is_err()
        );
        assert!(
            InputShareMessage::GrantHybrid {
                session_id: InputSessionId::new(),
                token: InputToken([0; 32]),
                ttl_ms: 1500,
                motion_enabled: true,
                pointer_mode: MotionPointerMode::Absolute,
                pointer_fallback: None,
            }
            .validate()
            .is_err()
        );
        assert!(
            InputShareMessage::GrantHybrid {
                session_id: InputSessionId::new(),
                token: InputToken::new(),
                ttl_ms: 0,
                motion_enabled: true,
                pointer_mode: MotionPointerMode::Absolute,
                pointer_fallback: None,
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn hybrid_transition_rejects_unnumbered_motion_at_every_position() {
        let boundary = SharedInputEvent::Button {
            button: 0,
            pressed: true,
        };
        for events in [
            vec![SharedInputEvent::Motion { x: 30., y: 40. }],
            vec![
                SharedInputEvent::Motion { x: 30., y: 40. },
                boundary.clone(),
            ],
            vec![
                boundary.clone(),
                SharedInputEvent::Motion { x: 30., y: 40. },
                boundary.clone(),
            ],
            vec![boundary, SharedInputEvent::Motion { x: 30., y: 40. }],
        ] {
            let update = HybridInputUpdate::Transition {
                position: hybrid_position(),
                events,
            };
            assert!(update.validate().is_err());
            assert!(
                InputShareMessage::Hybrid {
                    session_id: InputSessionId::new(),
                    token: InputToken::new(),
                    sequence: 3,
                    update
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn motion_debug_uses_redacted_token() {
        let motion = MotionDatagram {
            session_id: InputSessionId::new(),
            token: InputToken([19; 32]),
            state_epoch: 2,
            position: hybrid_position(),
        };
        let debug = format!("{motion:?}");
        assert!(debug.contains("InputToken([redacted])"));
        assert!(!debug.contains("19, 19"));
    }

    #[test]
    fn emergency_shortcut_migrates_old_settings_and_rejects_conflicts() {
        let mut value = serde_json::to_value(InputShortcutBindings::default()).unwrap();
        value.as_object_mut().unwrap().remove("emergency");
        let bindings: InputShortcutBindings = serde_json::from_value(value).unwrap();
        assert_eq!(bindings.emergency, default_emergency_shortcut());
        let mut settings = InputSharingSettings::default();
        settings.shortcuts.emergency = InputShortcut {
            key_code: 14,
            modifiers: 0x0c0000,
        };
        assert!(settings.validate().is_ok());
        for invalid in [
            settings.shortcuts.left.clone(),
            InputShortcut {
                key_code: 14,
                modifiers: 0,
            },
            InputShortcut {
                key_code: 14,
                modifiers: 0x020000,
            },
            InputShortcut {
                key_code: 128,
                modifiers: 0x0c0000,
            },
            InputShortcut {
                key_code: 2,
                modifiers: 0x120000,
            },
        ] {
            settings.shortcuts.emergency = invalid;
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn defaults_are_explicit_and_safe() {
        let settings = InputSharingSettings::default();
        assert!(!settings.enabled);
        assert_eq!(settings.role, InputShareRole::Disabled);
        assert_eq!(settings.switch_mode, EdgeSwitchMode::Instant);
        assert_eq!(settings.corner_size, 16);
        assert_eq!(settings.shortcuts.left.key_code, 123);
        assert_eq!(settings.shortcuts.left.modifiers, 0x1C0000);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn master_gate_defaults_on_for_legacy_settings_and_preserves_controller_configuration() {
        let legacy: InputSharingSettings = serde_json::from_str("{}").unwrap();
        assert!(legacy.feature_enabled);
        let mut snapshot = InputSharingSnapshot::default();
        snapshot.settings.feature_enabled = false;
        snapshot.settings.enabled = true;
        snapshot.settings.role = InputShareRole::Controller;
        let effective = snapshot.effective_settings();
        assert!(!effective.enabled);
        assert_eq!(effective.role, InputShareRole::Disabled);
        assert!(snapshot.settings.enabled);
        assert_eq!(snapshot.settings.role, InputShareRole::Controller);
    }

    #[test]
    fn legacy_scenes_without_a_network_binding_still_load() {
        let json = format!(
            r#"{{"id":"{}","name":"Office","settings":{{}}}}"#,
            Uuid::new_v4()
        );
        let scene: InputSharingScene = serde_json::from_str(&json).unwrap();
        assert_eq!(scene.ssid, None);
        assert!(scene.validate().is_ok());
    }
    #[test]
    fn permission_shape_and_coordinates_are_bounded() {
        let settings = InputSharingSettings {
            enabled: true,
            role: InputShareRole::Controlled,
            ..Default::default()
        };
        assert!(settings.validate().is_err());
        assert!(
            SharedInputEvent::Motion { x: f64::NAN, y: 1. }
                .validate()
                .is_err()
        );
        assert!(
            SharedInputEvent::Button {
                button: 32,
                pressed: true
            }
            .validate()
            .is_err()
        );
        assert!(
            SharedInputEvent::Key {
                code: 300,
                pressed: true,
                modifiers: 0,
                repeat: false
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn token_debug_never_contains_bytes() {
        assert_eq!(
            format!("{:?}", InputToken([4; 32])),
            "InputToken([redacted])"
        );
    }
    #[test]
    fn conflicting_or_unmodified_hotkeys_are_rejected() {
        let mut settings = InputSharingSettings::default();
        settings.shortcuts.left = settings.shortcuts.right.clone();
        assert!(settings.validate().is_err());
        settings = InputSharingSettings::default();
        settings.shortcuts.lock = InputShortcut {
            key_code: 53,
            modifiers: 0x1C0000,
        };
        assert!(settings.validate().is_err());
        settings = InputSharingSettings::default();
        settings.shortcuts.left.modifiers = 0;
        assert!(settings.validate().is_err());
    }
}

/// A named, independently persisted input configuration. Applying it always reclaims input first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InputSharingScene {
    pub id: String,
    pub name: String,
    /// When set, this scene is the remembered layout for that Wi-Fi network and
    /// is applied automatically when the stable environment returns to it.
    /// Scene bindings stay local; SSID is a switch key, never an authorization.
    #[serde(default)]
    pub ssid: Option<String>,
    pub settings: InputSharingSettings,
}

#[cfg(test)]
mod display_metadata_tests {
    use super::*;

    #[test]
    fn physical_size_is_optional_and_bounded() {
        let json = r#"{"generation":1,"displays":[{"id":"m","frame_x":0.0,"frame_y":0.0,"width":100,"height":100,"scale_factor":1.0,"rotation_degrees":0,"is_primary":true}]}"#;
        let topology: DisplayTopology = serde_json::from_str(json).unwrap();
        assert!(topology.validate().is_ok());
        assert!(topology.displays[0].physical_width_mm.is_none());

        let mut invalid = topology;
        invalid.displays[0].physical_width_mm = Some(f64::NAN);
        assert!(invalid.validate().is_err());
    }
}
impl InputSharingScene {
    pub fn new(name: String, settings: InputSharingSettings) -> Result<Self> {
        let scene = Self {
            id: Uuid::new_v4().to_string(),
            name: name.trim().to_owned(),
            ssid: None,
            settings,
        };
        scene.validate()?;
        Ok(scene)
    }
    pub fn validate(&self) -> Result<()> {
        if Uuid::parse_str(&self.id).is_err()
            || self.name.trim().is_empty()
            || self.name.chars().count() > 80
            || self.name.chars().any(char::is_control)
            || self
                .ssid
                .as_ref()
                .is_some_and(|ssid| ssid.is_empty() || ssid.len() > 32)
        {
            return Err(invalid("invalid input scene id, name or network binding"));
        }
        self.settings.validate()
    }
    pub fn validate_list(scenes: &[Self]) -> Result<()> {
        if scenes.len() > 16 {
            return Err(invalid("at most 16 input scenes are supported"));
        }
        let mut ids = BTreeSet::new();
        let mut networks = BTreeSet::new();
        for scene in scenes {
            scene.validate()?;
            if !ids.insert(&scene.id) {
                return Err(invalid("duplicate input scene id"));
            }
            if let Some(ssid) = &scene.ssid
                && !networks.insert(ssid.clone())
            {
                return Err(invalid("only one scene may be bound to a network"));
            }
        }
        Ok(())
    }
}
