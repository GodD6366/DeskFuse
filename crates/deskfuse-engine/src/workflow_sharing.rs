//! Sender-side workflow selection. No transformed output is committed to the sender's pasteboard.
use crate::workflow::{WorkflowInput, WorkflowOutputMode, WorkflowService};
use deskfuse_core::workflow::{Workflow, WorkflowContentType};
use deskfuse_platform::PasteboardSnapshot;
use std::sync::{
    Arc, Mutex, OnceLock, Weak,
    atomic::{AtomicBool, Ordering},
};
use std::{thread, time::Duration};

static SERVICE: OnceLock<Mutex<Weak<WorkflowService>>> = OnceLock::new();
pub fn attach(service: &Arc<WorkflowService>) {
    *SERVICE
        .get_or_init(|| Mutex::new(Weak::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::downgrade(service);
}
fn service() -> Result<Arc<WorkflowService>, String> {
    SERVICE
        .get()
        .and_then(|m| m.lock().ok()?.upgrade())
        .ok_or_else(|| "工作流引擎尚未就绪".into())
}
#[derive(Clone, Debug, Default)]
pub enum Selection {
    #[default]
    Automatic,
    Bypass,
    Id(String),
}
#[derive(Clone, Debug, Default)]
pub struct Selected {
    pub workflow: Option<Workflow>,
    pub error: Option<String>,
}
pub fn content_type(snapshot: &PasteboardSnapshot) -> WorkflowContentType {
    if !snapshot.file_urls.is_empty() {
        return WorkflowContentType::Files;
    }
    if snapshot.png.is_some()
        || snapshot
            .items
            .iter()
            .flat_map(|i| &i.representations)
            .any(|r| {
                matches!(
                    r.type_name.as_str(),
                    "public.png"
                        | "public.jpeg"
                        | "public.tiff"
                        | "public.heic"
                        | "public.heif"
                        | "org.webmproject.webp"
                )
            })
    {
        return WorkflowContentType::Image;
    }
    WorkflowContentType::Text
}
pub fn select(peer: &str, kind: WorkflowContentType, selection: &Selection) -> Selected {
    if matches!(selection, Selection::Bypass) {
        return Selected::default();
    }
    let result = (|| {
        let catalog = crate::workflow_catalog::global()?;
        let list = catalog.list()?;
        let id = match selection {
            Selection::Id(id) => Some(id.clone()),
            Selection::Automatic => {
                let view: serde_json::Value =
                    serde_json::from_str(&catalog.view_json()?).map_err(|e| e.to_string())?;
                let local = view["local_device_id"].as_str().unwrap_or("");
                let mut rules = catalog.rules()?;
                rules.retain(|r| {
                    r.enabled
                        && r.sender_id == local
                        && (r.target_id.is_empty() || r.target_id == peer)
                        && (r.content_type == WorkflowContentType::Any || r.content_type == kind)
                });
                rules.sort_by_key(|r| (r.target_id.is_empty(), r.position, r.id.clone()));
                rules.first().map(|r| r.workflow_id.clone())
            }
            Selection::Bypass => None,
        };
        match id {
            Some(id) => list
                .into_iter()
                .find(|w| w.id == id && w.enabled)
                .map(Some)
                .ok_or_else(|| "共享工作流不存在或已停用".to_owned()),
            None => Ok(None),
        }
    })();
    match result {
        Ok(workflow) => Selected {
            workflow,
            error: None,
        },
        Err(error) => Selected {
            workflow: None,
            error: Some(error),
        },
    }
}
pub struct ProcessedShare {
    pub snapshot: PasteboardSnapshot,
    pub paths: Vec<String>,
    pub warning: Option<String>,
    pub run_id: Option<String>,
}
/// Called exclusively by background preparation tasks. Cancellation never falls back.
pub fn process(
    selected: Selected,
    original: PasteboardSnapshot,
    cancel: Arc<AtomicBool>,
) -> Result<ProcessedShare, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("共享已取消".into());
    }
    let original_paths = original.file_urls.clone();
    let Some(definition) = selected.workflow else {
        return Ok(ProcessedShare {
            snapshot: original,
            paths: original_paths,
            warning: selected.error,
            run_id: None,
        });
    };
    let result = (|| {
        let service = service()?;
        let definition = crate::workflow_catalog::global()?.resolve_local(definition)?;
        let id = service.start_definition(
            definition,
            WorkflowInput::Snapshot(original.clone()),
            WorkflowOutputMode::ReturnResult,
            0,
            cancel.clone(),
        )?;
        loop {
            if cancel.load(Ordering::Acquire) {
                let _ = service.cancel(&id);
                return Err("共享已取消".into());
            }
            let run = service.status(&id)?;
            match run.phase.as_str() {
                "completed" => {
                    let snapshot = service.result_snapshot(&id)?;
                    let paths = run.output_paths;
                    return Ok(ProcessedShare {
                        snapshot,
                        paths,
                        warning: None,
                        run_id: Some(id),
                    });
                }
                "cancelled" => {
                    cancel.store(true, Ordering::Release);
                    return Err("共享已取消".into());
                }
                "failed" => return Err(run.message),
                _ => thread::sleep(Duration::from_millis(30)),
            }
        }
    })();
    match result {
        Ok(output) => Ok(output),
        Err(error) if cancel.load(Ordering::Acquire) => Err(error),
        Err(error) => Ok(ProcessedShare {
            snapshot: original,
            paths: original_paths,
            warning: Some(error),
            run_id: None,
        }),
    }
}
