//! Windows workflow admission and execution.
//! Definitions are still the `unidesk_core` model; this module only owns the
//! session-local input/output boundary and never treats JSON as execution.
use super::super::desktop_protocol::{Format, WorkflowInput};
use base64::Engine;
use serde::Deserialize;
use sha2::Digest;
use unidesk_core::workflow::{Workflow, WorkflowContentType, WorkflowStepKind};
use unidesk_store::SettingsStore;

const MAX_INPUT: usize = 16 * 1024 * 1024;

pub struct Result {
    pub run_id: String,
    pub formats: Vec<Format>,
    pub message: String,
}

pub fn execute(
    workflow: &Workflow,
    input: &[WorkflowInput],
) -> std::result::Result<Result, String> {
    if !workflow.enabled {
        return Err("工作流已停用".into());
    }
    workflow.validate()?;
    let text = input
        .iter()
        .find(|item| item.mime == "text/plain" || item.mime == "text/plain;charset=utf-8")
        .ok_or_else(|| "Windows 工作流当前只接受 text/plain 剪贴板输入".to_owned())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&text.data_base64)
        .map_err(|_| "工作流输入不是有效的 base64".to_owned())?;
    if bytes.len() > MAX_INPUT {
        return Err("工作流输入超过 16 MiB 上限".into());
    }
    let mut value = String::from_utf8(bytes).map_err(|_| "工作流输入不是 UTF-8 文字".to_owned())?;
    if !matches!(
        workflow.input_type,
        WorkflowContentType::Any | WorkflowContentType::Text
    ) {
        return Err("工作流声明的输入类型与 Windows 当前剪贴板不兼容".into());
    }
    for step in &workflow.steps {
        match step.kind {
            WorkflowStepKind::TextReplace => value = value.replace(&step.find, &step.replacement),
            WorkflowStepKind::TextTrim => value = value.trim().to_owned(),
            WorkflowStepKind::Copy | WorkflowStepKind::Paste => {}
            WorkflowStepKind::SystemShortcut => {
                return Err("Windows 尚未绑定此系统快捷指令，工作流未执行".into());
            }
            _ => return Err(format!("Windows 当前不支持 {:?} 步骤", step.kind)),
        }
        if value.len() > MAX_INPUT {
            return Err("工作流结果超过 16 MiB 上限".into());
        }
    }
    Ok(Result {
        run_id: uuid::Uuid::new_v4().to_string(),
        formats: vec![Format {
            mime: "text/plain".into(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(value.as_bytes()),
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
    store: unidesk_store::SQLiteStore,
    state: std::sync::Mutex<CatalogDocument>,
}
impl Catalog {
    pub fn open(
        store: unidesk_store::SQLiteStore,
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
    pub fn save_rule(
        &self,
        rule: unidesk_core::workflow::WorkflowRule,
    ) -> std::result::Result<(), String> {
        rule.validate()?;
        if !self
            .list()?
            .iter()
            .any(|workflow| workflow.id == rule.workflow_id)
        {
            return Err("规则引用的工作流不存在".into());
        }
        self.local_edit(
            format!("rule:{}", rule.id),
            Some(serde_json::json!({"kind":"Rule","value":rule})),
        )
    }
    pub fn delete_rule(&self, id: &str) -> std::result::Result<(), String> {
        self.local_edit(format!("rule:{id}"), None)
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
            state.records.insert(incoming.key, incoming);
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
