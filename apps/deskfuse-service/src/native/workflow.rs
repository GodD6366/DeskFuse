//! Windows workflow admission and execution.
//! Definitions are still the `deskfuse_core` model; this module only owns the
//! session-local input/output boundary and never treats JSON as execution.
use super::super::desktop_protocol::{Format, WorkflowInput};
use base64::Engine;
use deskfuse_core::workflow::{
    ImageFormat, Workflow, WorkflowContentType, WorkflowStep, WorkflowStepKind,
};
use deskfuse_engine::workflow_image;
use deskfuse_platform::{WorkflowCapabilities, WorkflowExecutionPlan, WorkflowOutputMode};
use serde::Deserialize;
use sha2::Digest;

const MAX_INPUT: usize = 16 * 1024 * 1024;
const MAX_IMAGE: usize = 128 * 1024 * 1024;

pub struct Result {
    pub run_id: String,
    pub formats: Vec<Format>,
    pub message: String,
}

enum Payload {
    Text(String),
    Image { bytes: Vec<u8>, format: ImageFormat },
}

struct ImagePolicy {
    quality: u8,
    target: Option<u64>,
    explicit: Option<ImageFormat>,
}

fn decode_input(item: &WorkflowInput) -> std::result::Result<Vec<u8>, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&item.data_base64)
        .map_err(|_| "工作流输入不是有效的 base64".to_owned())?;
    if bytes.len() > MAX_IMAGE {
        return Err("工作流输入超过 128 MiB 上限".into());
    }
    Ok(bytes)
}

fn is_text_mime(mime: &str) -> bool {
    mime == "text/plain" || mime.starts_with("text/plain;")
}

fn is_image_mime(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/webp" | "image/bmp" | "image/tiff" | "image/gif"
    )
}

fn has_text_steps(workflow: &Workflow) -> bool {
    workflow.steps.iter().any(|step| {
        matches!(
            step.kind,
            WorkflowStepKind::TextReplace | WorkflowStepKind::TextTrim
        )
    })
}

fn has_image_steps(workflow: &Workflow) -> bool {
    workflow.steps.iter().any(|step| {
        matches!(
            step.kind,
            WorkflowStepKind::Compress | WorkflowStepKind::Resize | WorkflowStepKind::Convert
        )
    })
}

fn decode_payload(
    workflow: &Workflow,
    input: &[WorkflowInput],
) -> std::result::Result<Payload, String> {
    if input
        .iter()
        .any(|item| item.mime == "application/x-deskfuse-local-files")
    {
        return Err("Windows 工作流暂不接受文件输入；请先把文件转换为文字或图片内容".into());
    }
    let text = || {
        input
            .iter()
            .find(|item| is_text_mime(&item.mime))
            .map(|item| {
                let bytes = decode_input(item)?;
                if bytes.len() > MAX_INPUT {
                    return Err("文字输入超过 16 MiB 上限".to_owned());
                }
                String::from_utf8(bytes).map_err(|_| "工作流输入不是 UTF-8 文字".to_owned())
            })
            .transpose()
    };
    let image = || {
        input
            .iter()
            .find(|item| is_image_mime(&item.mime))
            .map(|item| {
                let bytes = decode_input(item)?;
                let info = workflow_image::inspect_image(&bytes)?;
                let format = info.format.unwrap_or(if info.has_alpha {
                    ImageFormat::Png
                } else {
                    ImageFormat::Jpeg
                });
                Ok::<Payload, String>(Payload::Image { bytes, format })
            })
            .transpose()
    };
    let prefer_image = match workflow.input_type {
        WorkflowContentType::Text => false,
        WorkflowContentType::Image => true,
        WorkflowContentType::Files => {
            return Err("Windows 工作流暂不接受文件输入".into());
        }
        WorkflowContentType::Any => has_image_steps(workflow) || !has_text_steps(workflow),
    };
    let ordered = if prefer_image {
        image()?.or(text()?.map(Payload::Text))
    } else {
        text()?.map(Payload::Text).or(image()?)
    };
    ordered.ok_or_else(|| {
        if prefer_image {
            "剪贴板没有可处理的图片".into()
        } else {
            "剪贴板没有可处理的文字".into()
        }
    })
}

fn validate_input(kind: WorkflowContentType, payload: &Payload) -> std::result::Result<(), String> {
    let compatible = match (kind, payload) {
        (WorkflowContentType::Any, _) => true,
        (WorkflowContentType::Text, Payload::Text(_)) => true,
        (WorkflowContentType::Image, Payload::Image { .. }) => true,
        _ => false,
    };
    compatible
        .then_some(())
        .ok_or_else(|| "工作流声明的输入类型与 Windows 当前剪贴板不兼容".into())
}

fn image_step(
    payload: &mut Payload,
    step: &WorkflowStep,
    policy: &mut ImagePolicy,
) -> std::result::Result<(), String> {
    let Payload::Image { bytes, format } = payload else {
        return Err("图片步骤需要图片输入".into());
    };
    let info = workflow_image::inspect_image(bytes)?;
    let original = info.format.unwrap_or(if info.has_alpha {
        ImageFormat::Png
    } else {
        ImageFormat::Jpeg
    });
    if step.kind == WorkflowStepKind::Convert {
        if step.format == ImageFormat::Heic {
            return Err("Windows 不支持 HEIC 编码；请选择 PNG、JPEG 或 WebP".into());
        }
        policy.explicit = Some(step.format);
        policy.quality = step.quality;
    }
    if step.kind == WorkflowStepKind::Compress {
        if let Some(target) = step.target_bytes {
            policy.target = Some(policy.target.map_or(target, |old| old.min(target)));
        } else {
            policy.quality = step.quality;
        }
    }
    let target_format = match step.kind {
        WorkflowStepKind::Convert => step.format,
        WorkflowStepKind::Compress => {
            if info.has_alpha {
                ImageFormat::Png
            } else {
                ImageFormat::Jpeg
            }
        }
        _ => policy.explicit.unwrap_or(original),
    };
    if target_format == ImageFormat::Heic {
        return Err("Windows 不支持 HEIC 编码；请选择 PNG、JPEG 或 WebP".into());
    }
    // A target-volume compression is fitted once after every step so later
    // conversions cannot secretly exceed the declared ceiling.
    if step.kind == WorkflowStepKind::Compress && policy.target.is_some() {
        *format = target_format;
        return Ok(());
    }
    let edge = if step.kind == WorkflowStepKind::Resize {
        Some(step.max_edge.unwrap_or_else(|| {
            ((u64::from(info.width.max(info.height))
                * u64::from(step.scale_percent.unwrap_or(100)))
                / 100)
                .max(1) as u32
        }))
    } else {
        None
    };
    let result = workflow_image::encode_image(bytes, target_format, policy.quality, edge)?;
    if step.kind == WorkflowStepKind::Compress && result.data.len() >= bytes.len() {
        // Re-encoding did not help; keep the original bytes and format.
        *format = original;
        return Ok(());
    }
    *bytes = result.data;
    *format = target_format;
    Ok(())
}

fn fit_target(payload: &mut Payload, policy: &ImagePolicy) -> std::result::Result<(), String> {
    let Some(target) = policy.target else {
        return Ok(());
    };
    let Payload::Image { bytes, format } = payload else {
        return Ok(());
    };
    if bytes.len() as u64 <= target {
        return Ok(());
    }
    let source = bytes.clone();
    let info = workflow_image::inspect_image(&source)?;
    let target_format = policy
        .explicit
        .or(info.format)
        .unwrap_or(if info.has_alpha {
            ImageFormat::Png
        } else {
            ImageFormat::Jpeg
        });
    if target_format == ImageFormat::Heic {
        return Err("Windows 不支持 HEIC 编码；请选择 PNG、JPEG 或 WebP".into());
    }
    let original_edge = info.width.max(info.height);
    let floor = original_edge.min(256);
    let mut edge = original_edge;
    for _ in 0..32 {
        if target_format != ImageFormat::Png {
            let mut low = policy.quality.min(40);
            let mut high = policy.quality;
            let mut best = None;
            while low <= high {
                let mid = low + (high - low) / 2;
                let image = workflow_image::encode_image(&source, target_format, mid, Some(edge))?;
                if image.data.len() as u64 <= target {
                    best = Some(image);
                    low = mid.saturating_add(1);
                } else {
                    if mid == 0 {
                        break;
                    }
                    high = mid - 1;
                }
            }
            if let Some(image) = best {
                *bytes = image.data;
                *format = target_format;
                return Ok(());
            }
        } else {
            let image =
                workflow_image::encode_image(&source, target_format, policy.quality, Some(edge))?;
            if image.data.len() as u64 <= target {
                *bytes = image.data;
                *format = target_format;
                return Ok(());
            }
        }
        if edge == floor {
            break;
        }
        edge = ((u64::from(edge) * 80 / 100) as u32).max(floor);
    }
    Err("在最低质量和尺寸限制内仍无法达到最终体积，请增大目标上限".into())
}

pub fn execute(
    workflow: &Workflow,
    input: &[WorkflowInput],
) -> std::result::Result<Result, String> {
    // Admission is local and capability checked. The wire payload contains a
    // model/rule only; it never authorizes native execution by itself.
    for step in &workflow.steps {
        match step.kind {
            WorkflowStepKind::TextReplace | WorkflowStepKind::TextTrim => {
                if !matches!(
                    workflow.input_type,
                    WorkflowContentType::Any | WorkflowContentType::Text
                ) {
                    return Err("工作流声明的输入类型与文字步骤不兼容".into());
                }
            }
            WorkflowStepKind::Compress
            | WorkflowStepKind::Resize
            | WorkflowStepKind::Convert
            | WorkflowStepKind::Copy
            | WorkflowStepKind::Paste => {}
            WorkflowStepKind::SystemShortcut => {
                return Err("Windows 不提供 macOS 快捷指令步骤，流程未执行".into());
            }
            WorkflowStepKind::Rename | WorkflowStepKind::Zip => {
                return Err("Windows 工作流暂不支持重命名或 ZIP 文件步骤，流程未执行".into());
            }
        }
    }
    let plan = WorkflowExecutionPlan::new(workflow.clone(), WorkflowOutputMode::ReturnResult)
        .map_err(|error| error.to_string())?
        .authorize(&WorkflowCapabilities {
            steps: vec![
                WorkflowStepKind::TextReplace,
                WorkflowStepKind::TextTrim,
                WorkflowStepKind::Compress,
                WorkflowStepKind::Resize,
                WorkflowStepKind::Convert,
            ],
            clipboard_write: false,
            guarded_paste: false,
        })
        .map_err(|error| error.to_string())?;
    let workflow = plan.workflow();
    let mut payload = decode_payload(workflow, input)?;
    validate_input(workflow.input_type, &payload)?;
    let mut policy = ImagePolicy {
        quality: 85,
        target: None,
        explicit: None,
    };
    for step in &workflow.steps {
        match step.kind {
            WorkflowStepKind::TextReplace | WorkflowStepKind::TextTrim => {
                let Payload::Text(text) = &mut payload else {
                    return Err("文字步骤仅接受纯文字输入".into());
                };
                *text = if step.kind == WorkflowStepKind::TextTrim {
                    text.trim().to_owned()
                } else {
                    text.replace(&step.find, &step.replacement)
                };
                if text.len() > MAX_INPUT {
                    return Err("工作流结果超过 16 MiB 上限".into());
                }
            }
            WorkflowStepKind::Compress | WorkflowStepKind::Resize | WorkflowStepKind::Convert => {
                image_step(&mut payload, step, &mut policy)?
            }
            WorkflowStepKind::Copy | WorkflowStepKind::Paste => {}
            WorkflowStepKind::SystemShortcut | WorkflowStepKind::Rename | WorkflowStepKind::Zip => {
                return Err("Windows 当前不支持该工作流步骤".into());
            }
        }
    }
    fit_target(&mut payload, &policy)?;
    let (mime, bytes) = match payload {
        Payload::Text(value) => ("text/plain".to_owned(), value.into_bytes()),
        Payload::Image { bytes, format } => (format.mime().to_owned(), bytes),
    };
    Ok(Result {
        run_id: uuid::Uuid::new_v4().to_string(),
        formats: vec![Format {
            mime,
            data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        }],
        message: "工作流已在服务的受限会话中执行".into(),
    })
}

#[derive(Clone, serde::Serialize, serde::Deserialize, Default)]
struct CatalogDocument {
    local_device_id: String,
    counter: u64,
    records: std::collections::BTreeMap<String, CatalogRecord>,
    #[serde(default, deserialize_with = "discard_incompatible_conflicts")]
    conflicts: std::collections::BTreeMap<String, CatalogRecord>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct CatalogRecord {
    key: String,
    revision: Revision,
    clock: std::collections::BTreeMap<String, u64>,
    definition: Option<serde_json::Value>,
}
fn discard_incompatible_conflicts<'de, D>(
    deserializer: D,
) -> std::result::Result<std::collections::BTreeMap<String, CatalogRecord>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_object()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(id, value)| {
                    let record = value.get("record").unwrap_or(value);
                    serde_json::from_value(record.clone())
                        .ok()
                        .map(|record| (id.clone(), record))
                })
                .collect()
        })
        .unwrap_or_default())
}

#[derive(Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, PartialOrd, Ord)]
struct Revision {
    counter: u64,
    author: String,
}
#[derive(serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CatalogFrame<'a> {
    Summary {
        version: u32,
        device_id: &'a str,
        digest: String,
    },
    Record {
        record: &'a CatalogRecord,
    },
    Ack {
        digest: String,
    },
}
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum IncomingFrame {
    Summary {
        version: u32,
        device_id: String,
        digest: String,
    },
    Record {
        record: CatalogRecord,
    },
    Ack {
        digest: String,
    },
}

pub struct Catalog {
    store: deskfuse_store::SQLiteStore,
    state: std::sync::Mutex<CatalogDocument>,
}
impl Catalog {
    pub fn open(
        store: deskfuse_store::SQLiteStore,
        local_device_id: String,
    ) -> std::result::Result<Self, String> {
        let document = match store.workflow_catalog_json().map_err(|e| e.to_string())? {
            Some(json) => {
                serde_json::from_str(&json).map_err(|_| "工作流同步目录损坏".to_owned())?
            }
            None => {
                let mut document = CatalogDocument {
                    local_device_id: local_device_id.clone(),
                    ..Default::default()
                };
                for workflow in store.workflows().map_err(|e| e.to_string())? {
                    let key = format!("workflow:{}", workflow.id);
                    document.counter += 1;
                    let mut clock = std::collections::BTreeMap::new();
                    clock.insert(local_device_id.clone(), document.counter);
                    document.records.insert(
                        key.clone(),
                        CatalogRecord {
                            key,
                            revision: Revision {
                                counter: document.counter,
                                author: local_device_id.clone(),
                            },
                            clock,
                            definition: Some(
                                serde_json::json!({"kind":"Workflow","value":workflow}),
                            ),
                        },
                    );
                }
                document
            }
        };
        let mut document = document;
        document.local_device_id = local_device_id;
        let catalog = Self {
            store,
            state: std::sync::Mutex::new(document),
        };
        catalog.persist()?;
        Ok(catalog)
    }
    fn persist(&self) -> std::result::Result<(), String> {
        let state = self.state.lock().map_err(|_| "工作流同步目录不可用")?;
        self.store
            .set_workflow_catalog_json(&serde_json::to_string(&*state).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }
    pub fn list(&self) -> std::result::Result<Vec<Workflow>, String> {
        let state = self.state.lock().map_err(|_| "工作流同步目录不可用")?;
        state
            .records
            .values()
            .filter_map(|record| record.definition.as_ref())
            .filter_map(|value| {
                (value["kind"] == "Workflow")
                    .then(|| serde_json::from_value(value["value"].clone()).ok())
                    .flatten()
            })
            .collect::<Vec<_>>()
            .pipe(Ok)
    }
    pub fn save(&self, workflow: Workflow) -> std::result::Result<(), String> {
        workflow.validate()?;
        self.local_edit(
            format!("workflow:{}", workflow.id),
            Some(serde_json::json!({"kind":"Workflow","value":workflow})),
        )
    }
    pub fn delete(&self, id: &str) -> std::result::Result<(), String> {
        self.local_edit(format!("workflow:{id}"), None)
    }
    fn local_edit(
        &self,
        key: String,
        definition: Option<serde_json::Value>,
    ) -> std::result::Result<(), String> {
        let mut state = self.state.lock().map_err(|_| "工作流同步目录不可用")?;
        state.counter = state.counter.checked_add(1).ok_or("工作流修订号已耗尽")?;
        let mut clock = state
            .records
            .get(&key)
            .map(|r| r.clock.clone())
            .unwrap_or_default();
        clock.insert(state.local_device_id.clone(), state.counter);
        let record = CatalogRecord {
            key: key.clone(),
            revision: Revision {
                counter: state.counter,
                author: state.local_device_id.clone(),
            },
            clock,
            definition,
        };
        state.records.insert(key, record);
        drop(state);
        self.persist()
    }
    pub fn view_json(&self) -> std::result::Result<String, String> {
        let state = self.state.lock().map_err(|_| "工作流同步目录不可用")?;
        let digest = digest(&state)?;
        let workflows: Vec<_> = state
            .records
            .values()
            .filter_map(|r| r.definition.as_ref())
            .filter(|v| v["kind"] == "Workflow")
            .map(|v| v["value"].clone())
            .collect();
        let rules: Vec<_> = state
            .records
            .values()
            .filter_map(|r| r.definition.as_ref())
            .filter(|v| v["kind"] == "Rule")
            .map(|v| v["value"].clone())
            .collect();
        let conflicts: Vec<_> = state
            .conflicts
            .values()
            .map(|r| serde_json::json!({"id":conflict_id(r).unwrap_or_default(),"record":r}))
            .collect();
        serde_json::to_string(&serde_json::json!({"local_device_id":state.local_device_id,"sync_status":"pending","digest":digest,"workflows":workflows,"rules":rules,"conflicts":conflicts})).map_err(|e| e.to_string())
    }
    pub fn frames(&self) -> std::result::Result<Vec<Vec<u8>>, String> {
        let state = self.state.lock().map_err(|_| "工作流同步目录不可用")?;
        let mut frames = vec![
            serde_json::to_vec(&CatalogFrame::Summary {
                version: 2,
                device_id: &state.local_device_id,
                digest: digest(&state)?,
            })
            .map_err(|e| e.to_string())?,
        ];
        frames.extend(
            state
                .records
                .values()
                .map(|record| {
                    serde_json::to_vec(&CatalogFrame::Record { record }).map_err(|e| e.to_string())
                })
                .collect::<std::result::Result<Vec<_>, _>>()?,
        );
        Ok(frames)
    }
    pub fn receive(&self, bytes: &[u8]) -> std::result::Result<Vec<Vec<u8>>, String> {
        if bytes.len() > 128 * 1024 {
            return Err("工作流同步帧超过限制".into());
        }
        let incoming: IncomingFrame =
            serde_json::from_slice(bytes).map_err(|_| "工作流同步数据无效")?;
        match incoming {
            IncomingFrame::Summary { version: 2, .. } => self.frames(),
            IncomingFrame::Record { record } => {
                self.merge(record)?;
                Ok(Vec::new())
            }
            IncomingFrame::Ack { .. } => Ok(Vec::new()),
            IncomingFrame::Summary { .. } => Err("工作流同步版本不兼容".into()),
        }
    }
    fn merge(&self, incoming: CatalogRecord) -> std::result::Result<(), String> {
        let mut state = self.state.lock().map_err(|_| "工作流同步目录不可用")?;
        let Some(existing) = state.records.get(&incoming.key).cloned() else {
            state.records.insert(incoming.key.clone(), incoming);
            drop(state);
            return self.persist();
        };
        let incoming_wins = dominates(&incoming.clock, &existing.clock);
        let existing_wins = dominates(&existing.clock, &incoming.clock);
        if incoming_wins && !existing_wins {
            state.records.insert(incoming.key.clone(), incoming);
        } else if !incoming_wins && !existing_wins && incoming.definition != existing.definition {
            let (winner, loser) = if incoming.revision > existing.revision {
                (incoming, existing)
            } else {
                (existing, incoming)
            };
            state.records.insert(winner.key.clone(), winner);
            state.conflicts.insert(conflict_id(&loser)?, loser);
        }
        drop(state);
        self.persist()
    }
    pub fn restore(&self, id: &str) -> std::result::Result<(), String> {
        let mut state = self.state.lock().map_err(|_| "工作流同步目录不可用")?;
        let record = state.conflicts.remove(id).ok_or("冲突版本不存在")?;
        state.records.insert(record.key.clone(), record);
        drop(state);
        self.persist()
    }
}
fn dominates(
    a: &std::collections::BTreeMap<String, u64>,
    b: &std::collections::BTreeMap<String, u64>,
) -> bool {
    b.iter()
        .all(|(author, counter)| a.get(author).copied().unwrap_or_default() >= *counter)
}
fn digest(document: &CatalogDocument) -> std::result::Result<String, String> {
    Ok(hex::encode(sha2::Sha256::digest(
        serde_json::to_vec(&document.records).map_err(|e| e.to_string())?,
    )))
}
fn conflict_id(record: &CatalogRecord) -> std::result::Result<String, String> {
    Ok(hex::encode(sha2::Sha256::digest(
        serde_json::to_vec(record).map_err(|e| e.to_string())?,
    )))
}
trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}
