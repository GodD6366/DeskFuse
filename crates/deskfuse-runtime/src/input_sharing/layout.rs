//! Controller-only device placement. Geometry changes do not restart input adapters.
use super::*;
use deskfuse_input::layout::{
    LayoutRect, layout_displays, placements, remember_layout, snap_device_move,
};

#[derive(Default)]
pub(super) struct LayoutPersistence {
    pub(super) dirty: bool,
    pub(super) scene_dirty: bool,
    pub(super) pending: Option<tokio::task::JoinHandle<std::result::Result<(), RuntimeError>>>,
}

impl InputActor {
    fn layout_restore_diagnostic(&mut self, ssid: Option<&str>, outcome: &str) {
        if !deskfuse_core::debug_logging::enabled() {
            return;
        }
        let local = self.network.borrow().snapshot.local_device_id;
        let scene = ssid.and_then(|ssid| {
            self.snapshot
                .scenes
                .iter()
                .find(|s| s.ssid.as_deref() == Some(ssid))
        });
        let detail = format!(
            "ssid={ssid:?} outcome={outcome} scene_id={:?} placements={} profiles={} suspended={} managed={}",
            scene.map(|s| s.id.as_str()),
            self.snapshot.settings.placements.len(),
            self.snapshot.settings.layout_profiles.len(),
            self.snapshot.environment.local_suspended,
            self.snapshot.managed_control.is_some(),
        );
        self.activation_diagnostic(local, "wifi_layout_restore", || detail);
    }

    /// Return the Wi-Fi-bound scene after replacing its layout with `settings`.
    /// A first layout change on a network creates the scene, so the UI does not
    /// need a separate save action just to establish the binding.
    fn environment_scene_for(
        &self,
        settings: &InputSharingSettings,
    ) -> Option<Vec<InputSharingScene>> {
        // During a network transition the visible geometry can still belong to
        // the previous scene. Never overwrite a saved scene before restoring it.
        if self.environment_layout_pending() {
            return None;
        }
        let ssid = self.snapshot.environment.ssid.clone()?;
        let mut scenes = self.snapshot.scenes.clone();
        if let Some(scene) = scenes
            .iter_mut()
            .find(|scene| scene.ssid.as_deref() == Some(ssid.as_str()))
        {
            if scene.settings.placements == settings.placements
                && scene.settings.layout_profiles == settings.layout_profiles
            {
                return None;
            }
            scene.settings.placements = settings.placements.clone();
            scene.settings.layout_profiles = settings.layout_profiles.clone();
        } else {
            let mut scene = InputSharingScene::new(ssid.clone(), settings.clone()).ok()?;
            scene.ssid = Some(ssid);
            scenes.push(scene);
        }
        Some(scenes)
    }

    /// Keep the scene bound to the current network in step with the layout that
    /// is being persisted, so returning to this Wi-Fi restores this arrangement
    /// without a manual save.
    pub(super) fn refresh_environment_scene(&mut self) -> bool {
        let Some(scenes) = self.environment_scene_for(&self.snapshot.settings) else {
            return false;
        };
        self.snapshot.scenes = scenes;
        self.layout_persistence.scene_dirty = true;
        true
    }

    /// Apply the arrangement remembered for this network. Only the display
    /// arrangement travels with the network; sharing switches, participants and
    /// consent stay with the device, so a network change never turns input on.
    pub(super) fn apply_environment_layout(&mut self, ssid: Option<&str>) -> bool {
        if !self.snapshot.can_configure_layout() {
            self.layout_restore_diagnostic(ssid, "deferred_not_local_controller");
            return false;
        }
        let Some(scene) = ssid.and_then(|ssid| {
            self.snapshot
                .scenes
                .iter()
                .find(|scene| scene.ssid.as_deref() == Some(ssid))
        }) else {
            self.layout_restore_diagnostic(ssid, "no_saved_scene");
            return false;
        };
        let mut settings = self.snapshot.settings.clone();
        settings.placements = scene.settings.placements.clone();
        settings.layout_profiles = scene.settings.layout_profiles.clone();
        if settings == self.snapshot.settings {
            self.layout_restore_diagnostic(ssid, "already_current");
            return false;
        }
        if let Err(error) = settings.validate() {
            self.layout_restore_diagnostic(ssid, &format!("invalid_settings:{error}"));
            return false;
        }
        self.release_all("Wi-Fi environment layout changed");
        self.snapshot.settings = settings.clone();
        self.layout_persistence.dirty = true;
        self.emit(RuntimeEvent::InputSharingSettingsChanged(settings));
        self.layout_restore_diagnostic(ssid, "applied");
        true
    }
    // Capture before replacing native topology, and on idle for newly seen setups.
    // Missing peers never participate in the key or erase an existing profile.
    pub(super) fn remember_visible_layout(&mut self) {
        if !self.snapshot.can_configure_layout() {
            return;
        }
        let local = self.network.borrow().snapshot.local_device_id;
        let displays = layout_displays(&self.snapshot, local, false);
        let mut settings = self.snapshot.settings.clone();
        remember_layout(&mut settings, &self.snapshot, local, &displays, false);
        if settings != self.snapshot.settings && settings.validate().is_ok() {
            self.snapshot.settings = settings;
            self.layout_persistence.dirty = true;
        }
    }

    pub(super) fn start_layout_save(&mut self) {
        if !self.layout_persistence.dirty || self.layout_persistence.pending.is_some() {
            return;
        }
        let store = self.store.clone();
        let settings = self.snapshot.settings.clone();
        self.refresh_environment_scene();
        let scenes = std::mem::take(&mut self.layout_persistence.scene_dirty)
            .then(|| self.snapshot.scenes.clone());
        self.layout_persistence.dirty = false;
        self.layout_persistence.pending = Some(tokio::task::spawn_blocking(move || {
            store.set_input_sharing_settings(settings)?;
            if let Some(scenes) = scenes {
                store.set_input_sharing_scenes(scenes)?;
            }
            Ok(())
        }));
    }

    pub(super) async fn finish_layout_save(&mut self) {
        let Some(task) = self.layout_persistence.pending.take() else {
            return;
        };
        if !matches!(task.await, Ok(Ok(()))) {
            self.layout_persistence.dirty = true;
            self.layout_persistence.scene_dirty = true;
            self.error("屏幕组合排布未能保存，将自动重试");
        } else {
            self.layout_persistence.scene_dirty = false;
        }
    }

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
        let mut displays = layout_displays(&self.snapshot, local, false);
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
        for display in &mut displays {
            if display.key.device_id == device_id {
                display.x += dx;
                display.y += dy;
            }
        }
        let resolved = placements(&displays);
        remember_layout(&mut settings, &self.snapshot, local, &displays, true);
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
        // Serialize full-settings writes with background layout persistence.
        self.finish_layout_save().await;
        let store = self.store.clone();
        let saved = settings.clone();
        // Keep the scene bound to this network in step with the arrangement.
        let scenes = self.environment_scene_for(&settings);
        let applied_scenes = scenes.clone();
        if matches!(
            tokio::task::spawn_blocking(move || -> std::result::Result<(), RuntimeError> {
                store.set_input_sharing_settings(saved)?;
                if let Some(scenes) = scenes {
                    store.set_input_sharing_scenes(scenes)?;
                }
                Ok(())
            })
            .await,
            Ok(Ok(()))
        ) {
            self.layout_persistence.dirty = false;
            self.layout_persistence.scene_dirty = false;
            self.snapshot.settings = settings.clone();
            if let Some(scenes) = applied_scenes {
                self.snapshot.scenes = scenes;
            }
            // Re-routing an active pointer is handled by the native coordinator, not an
            // availability toggle or a new controller intent. Pre-granted leases survive.
            self.emit(RuntimeEvent::InputSharingSettingsChanged(settings));
        } else {
            self.error("屏幕布局未能保存");
        }
    }
}
