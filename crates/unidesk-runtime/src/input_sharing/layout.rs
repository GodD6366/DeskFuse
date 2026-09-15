//! Controller-only device placement. Geometry changes do not restart input adapters.
use super::*;
use unidesk_input::layout::{LayoutRect, layout_displays, placements, snap_device_move};

impl InputActor {
    pub(super) async fn move_layout(
        &mut self,
        device_id: DeviceId,
        display_id: DisplayId,
        x: f64,
        y: f64,
    ) {
        if !self.snapshot.can_configure_layout() {
            self.error("只能在主控电脑上调整屏幕布局");
            return;
        }
        if !x.is_finite() || !y.is_finite() || x.abs() > 1_000_000.0 || y.abs() > 1_000_000.0 {
            self.error("无效的屏幕位置");
            return;
        }
        let local = self.network.borrow().snapshot.local_device_id;
        let displays = layout_displays(&self.snapshot, local, false);
        let Some(anchor) = displays
            .iter()
            .find(|d| d.key.device_id == device_id && d.key.display_id == display_id)
        else {
            self.error("设备显示器已断开，请重新调整布局");
            return;
        };
        let rects: Vec<_> = displays.iter().map(LayoutRect::from).collect();
        let (dx, dy) = match snap_device_move(&rects, device_id, x - anchor.x, y - anchor.y, None) {
            Ok(delta) => delta,
            Err(error) => {
                self.error(error);
                return;
            }
        };
        let mut settings = self.snapshot.settings.clone();
        let mut resolved = placements(&displays);
        for p in &mut resolved {
            if p.display.device_id == device_id {
                p.x += dx;
                p.y += dy;
            }
        }
        // Keep offline devices' saved positions, but freeze every visible group's current origin.
        settings
            .placements
            .retain(|p| !resolved.iter().any(|r| r.display == p.display));
        settings.placements.extend(resolved);
        if let Err(error) = settings.validate() {
            self.error(error.to_string());
            return;
        }
        if settings == self.snapshot.settings {
            return;
        }
        let store = self.store.clone();
        let saved = settings.clone();
        if matches!(
            tokio::task::spawn_blocking(move || store.set_input_sharing_settings(saved)).await,
            Ok(Ok(()))
        ) {
            self.snapshot.settings = settings.clone();
            // Re-routing an active pointer is handled by the native coordinator, not an
            // availability toggle or a new controller intent. Pre-granted leases survive.
            self.emit(RuntimeEvent::InputSharingSettingsChanged(settings));
        } else {
            self.error("屏幕布局未能保存");
        }
    }
}
