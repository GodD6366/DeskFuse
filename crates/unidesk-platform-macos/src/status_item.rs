use crate::PlatformResult;

/// State used by the status-item boundary.  It contains no UI objects and can
/// therefore be shared with non-GPUI frontends and deterministic tests.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatusItemState {
    pub visible: bool,
    pub title: String,
    pub tooltip: String,
}

/// Narrow status-item contract for a future native macOS implementation.
pub trait StatusItemController: Send {
    fn set_visible(&mut self, visible: bool) -> PlatformResult<()>;
    fn set_title(&mut self, title: &str) -> PlatformResult<()>;
    fn set_tooltip(&mut self, tooltip: &str) -> PlatformResult<()>;

    fn show(&mut self) -> PlatformResult<()> {
        self.set_visible(true)
    }

    fn hide(&mut self) -> PlatformResult<()> {
        self.set_visible(false)
    }
}

/// Explicit no-op status item for tests and unsupported targets.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopStatusItemController {
    visible: bool,
}

impl NoopStatusItemController {
    pub const fn new() -> Self {
        Self { visible: false }
    }

    pub const fn is_visible(&self) -> bool {
        self.visible
    }
}

impl StatusItemController for NoopStatusItemController {
    fn set_visible(&mut self, visible: bool) -> PlatformResult<()> {
        self.visible = visible;
        Ok(())
    }

    fn set_title(&mut self, _title: &str) -> PlatformResult<()> {
        Ok(())
    }

    fn set_tooltip(&mut self, _tooltip: &str) -> PlatformResult<()> {
        Ok(())
    }
}

/// In-memory status item used to assert state transitions without creating a
/// menu bar object.
#[derive(Debug, Default, Clone)]
pub struct MemoryStatusItemController {
    state: StatusItemState,
}

impl MemoryStatusItemController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> &StatusItemState {
        &self.state
    }

    pub fn into_state(self) -> StatusItemState {
        self.state
    }
}

impl StatusItemController for MemoryStatusItemController {
    fn set_visible(&mut self, visible: bool) -> PlatformResult<()> {
        self.state.visible = visible;
        Ok(())
    }

    fn set_title(&mut self, title: &str) -> PlatformResult<()> {
        self.state.title.clear();
        self.state.title.push_str(title);
        Ok(())
    }

    fn set_tooltip(&mut self, tooltip: &str) -> PlatformResult<()> {
        self.state.tooltip.clear();
        self.state.tooltip.push_str(tooltip);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_status_item_tracks_state() {
        let mut item = MemoryStatusItemController::new();
        item.show().expect("memory status item is available");
        item.set_title("UniDesk")
            .expect("memory status item is available");
        item.set_tooltip("Ready")
            .expect("memory status item is available");

        assert_eq!(
            item.state(),
            &StatusItemState {
                visible: true,
                title: "UniDesk".into(),
                tooltip: "Ready".into(),
            }
        );
    }
}
