//! Durable workflow definitions and bounded anti-entropy over trusted peer sessions.
//! Execution data never enters this catalog. Native shortcut bindings stay local.
use deskfuse_core::workflow::{Workflow, WorkflowRule, WorkflowStepKind};
use deskfuse_identity::FileIdentityProvider;
use deskfuse_store::SQLiteStore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

const MAX_FRAME: usize = 128 * 1024;
const MAX_RECORDS: usize = 2048;
const MAX_CONFLICTS: usize = 256;
const MAX_DOCUMENT: usize = 64 * 1024 * 1024;
const TOKEN_PREFIX: &str = "deskfuse-workflow-step:";
type Clock = BTreeMap<String, u64>;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct Revision {
    counter: u64,
    author: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
enum Definition {
    Workflow(Workflow),
    Rule(WorkflowRule),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Record {
    key: String,
    revision: Revision,
    clock: Clock,
    /// Deletion is retained as a versioned record, not absence.
    definition: Option<Definition>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Conflict {
    id: String,
    record: Record,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Binding {
    workflow_id: String,
    token: String,
    shortcut_id: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Document {
    local_device_id: String,
    counter: u64,
    records: BTreeMap<String, Record>,
    conflicts: BTreeMap<String, Conflict>,
    bindings: Vec<Binding>,
}
#[derive(Default)]
struct State {
    document: Document,
    /// Peer values are exact acknowledged content hashes, never merely send status.
    peers: BTreeMap<String, Option<String>>,
    last_error: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Message {
    Summary {
        version: u32,
        device_id: String,
        digest: String,
    },
    Request {
        keys: Option<Vec<String>>,
    },
    Record {
        record: Record,
    },
    Ack {
        digest: String,
    },
}

pub struct Catalog {
    store: SQLiteStore,
    state: Mutex<State>,
    generation: AtomicU64,
}
static GLOBAL: OnceLock<Arc<Catalog>> = OnceLock::new();
static INITIALIZE: Mutex<()> = Mutex::new(());

pub fn global() -> Result<Arc<Catalog>, String> {
    if let Some(catalog) = GLOBAL.get() {
        return Ok(catalog.clone());
    }
    let _guard = INITIALIZE.lock().map_err(|_| "工作流目录初始化不可用")?;
    if let Some(catalog) = GLOBAL.get() {
        return Ok(catalog.clone());
    }
    let store = SQLiteStore::new().map_err(|e| e.to_string())?;
    let identity = FileIdentityProvider::new(
        SQLiteStore::default_path()
            .map_err(|e| e.to_string())?
            .with_file_name("identity.key"),
    )
    .load_or_create()
    .map_err(|e| e.to_string())?;
    let local_device_id = identity.device_id().to_string();
    let persisted = store.workflow_catalog_json().map_err(|e| e.to_string())?;
    let mut document = match &persisted {
        Some(json) if json.len() <= MAX_DOCUMENT => serde_json::from_str::<Document>(json)
            .map_err(|_| "工作流目录损坏，未覆盖已有数据".to_string())?,
        Some(_) => return Err("工作流目录超过容量限制".into()),
        None => Document::default(),
    };
    document.local_device_id = local_device_id;
    validate_document(&document)?;
    let catalog = Arc::new(Catalog {
        store,
        state: Mutex::new(State {
            document,
            ..State::default()
        }),
        generation: AtomicU64::new(1),
    });
    if persisted.is_none() {
        let migrated = catalog.store.workflows().map_err(|e| e.to_string())?;
        // Capture native ids locally before stripping them from synchronized definitions.
        let mut state = catalog.state.lock().map_err(|_| "工作流目录不可用")?;
        let mut next = state.document.clone();
        for mut workflow in migrated {
            prepare_workflow(&mut next, &mut workflow)?;
            put_local(
                &mut next,
                workflow_key(&workflow.id),
                Some(Definition::Workflow(workflow)),
            )?;
        }
        catalog.commit(&mut state, next)?;
    }
    let _ = GLOBAL.set(catalog.clone());
    Ok(catalog)
}

impl Catalog {
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn set_local_device(&self, id: String) -> Result<(), String> {
        valid_id(&id)?;
        let mut state = self.state.lock().map_err(|_| "工作流目录不可用")?;
        if state.document.local_device_id != id {
            let mut next = state.document.clone();
            next.local_device_id = id;
            self.commit(&mut state, next)?;
        }
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<Workflow>, String> {
        let state = self.state.lock().map_err(|_| "工作流目录不可用")?;
        Ok(state
            .document
            .records
            .values()
            .filter_map(|r| match &r.definition {
                Some(Definition::Workflow(workflow)) => Some(workflow.clone()),
                _ => None,
            })
            .collect())
    }

    pub fn save(&self, mut workflow: Workflow) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| "工作流目录不可用")?;
        let mut next = state.document.clone();
        prepare_workflow(&mut next, &mut workflow)?;
        put_local(
            &mut next,
            workflow_key(&workflow.id),
            Some(Definition::Workflow(workflow)),
        )?;
        self.commit(&mut state, next)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        valid_id(id)?;
        let mut state = self.state.lock().map_err(|_| "工作流目录不可用")?;
        let mut next = state.document.clone();
        put_local(&mut next, workflow_key(id), None)?;
        next.bindings.retain(|b| b.workflow_id != id);
        self.commit(&mut state, next)
    }

    pub fn rules(&self) -> Result<Vec<WorkflowRule>, String> {
        let state = self.state.lock().map_err(|_| "工作流目录不可用")?;
        let mut rules: Vec<_> = state
            .document
            .records
            .values()
            .filter_map(|r| match &r.definition {
                Some(Definition::Rule(rule)) => Some(rule.clone()),
                _ => None,
            })
            .collect();
        rules.sort_by(|a, b| (a.position, &a.id).cmp(&(b.position, &b.id)));
        Ok(rules)
    }

    pub fn save_rule(&self, rule: WorkflowRule) -> Result<(), String> {
        rule.validate()?;
        let mut state = self.state.lock().map_err(|_| "工作流目录不可用")?;
        let mut next = state.document.clone();
        if !next
            .records
            .get(&workflow_key(&rule.workflow_id))
            .is_some_and(|r| matches!(r.definition, Some(Definition::Workflow(_))))
        {
            return Err("规则引用的工作流不存在".into());
        }
        put_local(&mut next, rule_key(&rule.id), Some(Definition::Rule(rule)))?;
        self.commit(&mut state, next)
    }

    pub fn delete_rule(&self, id: &str) -> Result<(), String> {
        valid_id(id)?;
        let mut state = self.state.lock().map_err(|_| "工作流目录不可用")?;
        let mut next = state.document.clone();
        put_local(&mut next, rule_key(id), None)?;
        self.commit(&mut state, next)
    }

    pub fn bind_shortcut(
        &self,
        workflow_id: &str,
        step_index: u32,
        shortcut_id: String,
    ) -> Result<(), String> {
        if shortcut_id.is_empty()
            || shortcut_id.len() > 256
            || shortcut_id.starts_with(TOKEN_PREFIX)
        {
            return Err("系统快捷指令标识无效".into());
        }
        let mut state = self.state.lock().map_err(|_| "工作流目录不可用")?;
        let mut next = state.document.clone();
        let workflow = match next
            .records
            .get(&workflow_key(workflow_id))
            .and_then(|r| r.definition.as_ref())
        {
            Some(Definition::Workflow(workflow)) => workflow,
            _ => return Err("工作流不存在".into()),
        };
        let step = workflow
            .steps
            .get(step_index as usize)
            .filter(|s| s.kind == WorkflowStepKind::SystemShortcut)
            .ok_or("步骤不是系统快捷指令")?;
        let token = step.shortcut_id.clone();
        next.bindings
            .retain(|b| !(b.workflow_id == workflow_id && b.token == token));
        next.bindings.push(Binding {
            workflow_id: workflow_id.into(),
            token,
            shortcut_id,
        });
        self.commit(&mut state, next)
    }

    /// Resolve tokens only through this device's explicit bindings. Names never imply trust.
    pub fn resolve_local(&self, mut workflow: Workflow) -> Result<Workflow, String> {
        let state = self.state.lock().map_err(|_| "工作流目录不可用")?;
        for step in &mut workflow.steps {
            if step.kind != WorkflowStepKind::SystemShortcut {
                continue;
            }
            let binding = state
                .document
                .bindings
                .iter()
                .find(|b| b.workflow_id == workflow.id && b.token == step.shortcut_id)
                .ok_or_else(|| format!("本机尚未绑定系统快捷指令：{}", step.shortcut_name))?;
            step.shortcut_id.clone_from(&binding.shortcut_id);
        }
        Ok(workflow)
    }

    pub fn restore_conflict(&self, id: &str) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| "工作流目录不可用")?;
        let mut next = state.document.clone();
        let conflict = next.conflicts.get(id).cloned().ok_or("冲突版本不存在")?;
        put_local(&mut next, conflict.record.key, conflict.record.definition)?;
        next.conflicts.remove(id);
        self.commit(&mut state, next)
    }

    pub fn refresh_peers(&self, peers: Vec<String>) -> Result<(), String> {
        let mut state = self.state.lock().map_err(|_| "工作流目录不可用")?;
        let ids: BTreeSet<_> = peers
            .into_iter()
            .filter(|p| valid_id(p).is_ok() && p != &state.document.local_device_id)
            .take(128)
            .collect();
        let before: BTreeSet<_> = state.peers.keys().cloned().collect();
        state.peers.retain(|id, _| ids.contains(id));
        for id in &ids {
            state.peers.entry(id.clone()).or_default();
        }
        if before != ids {
            self.generation.fetch_add(1, Ordering::AcqRel);
        }
        Ok(())
    }

    pub fn manifest(&self) -> Result<Vec<u8>, String> {
        let state = self.state.lock().map_err(|_| "工作流目录不可用")?;
        encode(&Message::Summary {
            version: 2,
            device_id: state.document.local_device_id.clone(),
            digest: digest(&state.document)?,
        })
    }

    /// Compatibility entrypoint; callers should prefer authenticated receive_from.
    pub fn receive(&self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        self.receive_from("", bytes)
    }

    pub fn receive_from(&self, peer_id: &str, bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        let result = self.receive_inner(peer_id, bytes);
        if let Err(error) = &result
            && let Ok(mut state) = self.state.lock()
            && state.last_error.as_ref() != Some(error)
        {
            state.last_error = Some(error.clone());
            self.generation.fetch_add(1, Ordering::AcqRel);
        }
        result
    }

    fn receive_inner(&self, peer_id: &str, bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        if bytes.len() > MAX_FRAME {
            return Err("工作流同步帧超过限制".into());
        }
        if !peer_id.is_empty() {
            valid_id(peer_id)?;
        }
        let message: Message = serde_json::from_slice(bytes).map_err(|_| "工作流同步数据无效")?;
        let mut state = self.state.lock().map_err(|_| "工作流目录不可用")?;
        let mut responses = Vec::new();
        match message {
            Message::Summary {
                version,
                device_id,
                digest: remote_digest,
            } => {
                valid_id(&device_id)?;
                if version != 2 || (!peer_id.is_empty() && peer_id != device_id) {
                    return Err("工作流同步版本或设备不匹配".into());
                }
                valid_digest(&remote_digest)?;
                let local_digest = digest(&state.document)?;
                if remote_digest == local_digest {
                    responses.push(encode(&Message::Ack {
                        digest: local_digest,
                    })?);
                } else {
                    if !peer_id.is_empty() {
                        acknowledge(&mut state, peer_id, "", &self.generation);
                    }
                    // Exchange individual records. A catalog is never serialized as a giant frame.
                    for record in state.document.records.values() {
                        responses.push(encode(&Message::Record {
                            record: record.clone(),
                        })?);
                    }
                    responses.push(encode(&Message::Request { keys: None })?);
                }
            }
            Message::Request { keys } => {
                if keys.as_ref().is_some_and(|keys| {
                    keys.len() > MAX_RECORDS || keys.iter().any(|k| k.len() > 140)
                }) {
                    return Err("工作流同步请求超过限制".into());
                }
                for record in state.document.records.values() {
                    if keys.as_ref().is_none_or(|keys| keys.contains(&record.key)) {
                        responses.push(encode(&Message::Record {
                            record: record.clone(),
                        })?);
                    }
                }
                responses.push(encode(&Message::Ack {
                    digest: digest(&state.document)?,
                })?);
            }
            Message::Record { record } => {
                validate_record(&record)?;
                let mut next = state.document.clone();
                if merge_record(&mut next, record)? {
                    self.commit(&mut state, next)?;
                }
                responses.push(encode(&Message::Ack {
                    digest: digest(&state.document)?,
                })?);
            }
            Message::Ack {
                digest: remote_digest,
            } => {
                valid_digest(&remote_digest)?;
                if !peer_id.is_empty() {
                    acknowledge(&mut state, peer_id, &remote_digest, &self.generation);
                }
            }
        }
        if state.last_error.take().is_some() {
            self.generation.fetch_add(1, Ordering::AcqRel);
        }
        Ok(responses)
    }

    pub fn view_json(&self) -> Result<String, String> {
        let state = self.state.lock().map_err(|_| "工作流目录不可用")?;
        let current_digest = digest(&state.document)?;
        let rules: Vec<_> = state
            .document
            .records
            .values()
            .filter_map(|r| match &r.definition {
                Some(Definition::Rule(rule)) => Some(rule),
                _ => None,
            })
            .collect();
        let conflicts: Vec<_> = state.document.conflicts.values().map(|conflict| {
            let (name, workflow) = match &conflict.record.definition {
                Some(Definition::Workflow(workflow)) => (workflow.name.clone(), Some(workflow)),
                Some(Definition::Rule(rule)) => (format!("共享规则 {}", rule.id), None),
                None => (conflict.record.key.clone(), None),
            };
            serde_json::json!({"id":conflict.id,"name":name,"workflow":workflow,"message":"设备并发修改，未采用的版本已保留，可恢复"})
        }).collect();
        let bindings: Vec<_> = state.document.records.values().flat_map(|r| match &r.definition {
            Some(Definition::Workflow(workflow)) => workflow.steps.iter().enumerate().filter_map(|(i, step)| {
                if step.kind != WorkflowStepKind::SystemShortcut { return None; }
                let id = state.document.bindings.iter().find(|b| b.workflow_id == workflow.id && b.token == step.shortcut_id).map(|b| b.shortcut_id.as_str()).unwrap_or("");
                Some(serde_json::json!({"workflow_id":workflow.id,"step_index":i,"shortcut_id":id}))
            }).collect::<Vec<_>>(), _ => Vec::new(),
        }).collect();
        let peers: Vec<_> = state.peers.iter().map(|(id, ack)| serde_json::json!({"device_id":id,"status":if ack.as_deref() == Some(current_digest.as_str()) {"synced"} else {"pending"}})).collect();
        let status = if state.last_error.is_some() {
            "error"
        } else if state.peers.is_empty() {
            "waiting_for_devices"
        } else if state
            .peers
            .values()
            .all(|ack| ack.as_deref() == Some(current_digest.as_str()))
        {
            "synced"
        } else {
            "pending"
        };
        serde_json::to_string(&serde_json::json!({"local_device_id":state.document.local_device_id,"sync_status":status,"generation":self.generation(),"rules":rules,"conflicts":conflicts,"bindings":bindings,"peers":peers,"error":state.last_error})).map_err(|e| e.to_string())
    }

    fn commit(&self, state: &mut State, next: Document) -> Result<(), String> {
        validate_document(&next)?;
        let json = serde_json::to_string(&next).map_err(|e| e.to_string())?;
        if json.len() > MAX_DOCUMENT {
            return Err("工作流目录容量已满".into());
        }
        self.store
            .set_workflow_catalog_json(&json)
            .map_err(|e| e.to_string())?;
        state.document = next;
        self.generation.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
}

fn workflow_key(id: &str) -> String {
    format!("workflow:{id}")
}
fn rule_key(id: &str) -> String {
    format!("rule:{id}")
}
fn valid_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
        Err("工作流标识无效".into())
    } else {
        Ok(())
    }
}
fn valid_digest(value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        Err("工作流版本摘要无效".into())
    } else {
        Ok(())
    }
}
fn encode(message: &Message) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(message).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FRAME {
        Err("工作流定义过大，单个定义不能超过 128 KiB".into())
    } else {
        Ok(bytes)
    }
}
fn digest(document: &Document) -> Result<String, String> {
    let bytes = serde_json::to_vec(&document.records).map_err(|e| e.to_string())?;
    Ok(hex::encode(Sha256::digest(bytes)))
}
fn acknowledge(state: &mut State, peer: &str, digest: &str, generation: &AtomicU64) {
    if !state.peers.contains_key(peer) && state.peers.len() >= 128 {
        return;
    }
    let value = Some(digest.to_owned());
    if state.peers.get(peer) != Some(&value) {
        state.peers.insert(peer.into(), value);
        generation.fetch_add(1, Ordering::AcqRel);
    }
}

fn prepare_workflow(document: &mut Document, workflow: &mut Workflow) -> Result<(), String> {
    workflow.version = 2;
    workflow.validate()?;
    valid_id(&workflow.id)?;
    let mut tokens = BTreeSet::new();
    for (index, step) in workflow.steps.iter_mut().enumerate() {
        if step.kind != WorkflowStepKind::SystemShortcut {
            continue;
        }
        if step.shortcut_name.len() > 1024 {
            return Err("系统快捷指令名称过长".into());
        }
        if !step.shortcut_id.starts_with(TOKEN_PREFIX) {
            let native_id = step.shortcut_id.clone();
            // Preserve an existing token when a native picker returns the same binding.
            let token = document
                .bindings
                .iter()
                .find(|b| {
                    b.workflow_id == workflow.id
                        && b.shortcut_id == native_id
                        && !tokens.contains(&b.token)
                })
                .map(|b| b.token.clone())
                .unwrap_or_else(|| {
                    let seed = format!(
                        "{}:{}:{}:{}:{}",
                        document.local_device_id,
                        document.counter.saturating_add(1),
                        workflow.id,
                        index,
                        native_id
                    );
                    format!(
                        "{TOKEN_PREFIX}{}",
                        hex::encode(Sha256::digest(seed.as_bytes()))
                    )
                });
            document
                .bindings
                .retain(|b| !(b.workflow_id == workflow.id && b.token == token));
            document.bindings.push(Binding {
                workflow_id: workflow.id.clone(),
                token: token.clone(),
                shortcut_id: native_id,
            });
            step.shortcut_id = token;
        }
        if !tokens.insert(step.shortcut_id.clone()) {
            return Err("系统快捷指令步骤标识重复，请重新选择快捷指令".into());
        }
    }
    // Keep removed-step bindings for recovery, but never bind them by array index.
    Ok(())
}
fn put_local(
    document: &mut Document,
    key: String,
    definition: Option<Definition>,
) -> Result<(), String> {
    let mut clock = document
        .records
        .get(&key)
        .map(|r| r.clock.clone())
        .unwrap_or_default();
    // Explicit local edits observe every preserved branch, including a restored loser.
    // This prevents a later stale branch from competing with an intentional resolution.
    for conflict in document.conflicts.values().filter(|c| c.record.key == key) {
        for (author, counter) in &conflict.record.clock {
            let value = clock.entry(author.clone()).or_default();
            *value = (*value).max(*counter);
        }
    }
    document.counter = document
        .counter
        .checked_add(1)
        .ok_or("工作流修订号已耗尽")?;
    clock.insert(document.local_device_id.clone(), document.counter);
    let record = Record {
        key: key.clone(),
        revision: Revision {
            counter: document.counter,
            author: document.local_device_id.clone(),
        },
        clock,
        definition,
    };
    validate_record(&record)?;
    if !document.records.contains_key(&key) && document.records.len() >= MAX_RECORDS {
        return Err("工作流目录已达到容量限制".into());
    }
    document.records.insert(key, record);
    Ok(())
}
fn dominates(a: &Clock, b: &Clock) -> bool {
    b.iter()
        .all(|(author, counter)| a.get(author).copied().unwrap_or(0) >= *counter)
}
fn backup_conflict(document: &mut Document, record: Record) -> Result<(), String> {
    let id = hex::encode(Sha256::digest(
        serde_json::to_vec(&record).map_err(|e| e.to_string())?,
    ));
    if document.conflicts.contains_key(&id) {
        return Ok(());
    }
    if document.conflicts.len() >= MAX_CONFLICTS {
        return Err("工作流冲突记录已满，请先恢复或处理现有冲突".into());
    }
    document
        .conflicts
        .insert(id.clone(), Conflict { id, record });
    Ok(())
}
fn merge_record(document: &mut Document, incoming: Record) -> Result<bool, String> {
    document.counter = document.counter.max(incoming.revision.counter);
    let Some(existing) = document.records.get(&incoming.key).cloned() else {
        if document.records.len() >= MAX_RECORDS {
            return Err("工作流目录已达到容量限制".into());
        }
        document.records.insert(incoming.key.clone(), incoming);
        return Ok(true);
    };
    let incoming_wins = dominates(&incoming.clock, &existing.clock);
    let existing_wins = dominates(&existing.clock, &incoming.clock);
    if incoming_wins && existing_wins {
        if serde_json::to_vec(&incoming).map_err(|e| e.to_string())?
            != serde_json::to_vec(&existing).map_err(|e| e.to_string())?
        {
            return Err("工作流相同版本含不同内容，拒绝覆盖".into());
        }
        return Ok(false);
    }
    if existing_wins {
        return Ok(false);
    }
    if incoming_wins {
        document.records.insert(incoming.key.clone(), incoming);
        return Ok(true);
    }
    // Concurrent branches converge by Lamport counter then stable author id. Keep the loser.
    let before_conflicts = document.conflicts.len();
    let (winner, loser) = if incoming.revision > existing.revision {
        (incoming, existing)
    } else {
        (existing, incoming)
    };
    if serde_json::to_vec(&winner.definition).map_err(|e| e.to_string())?
        != serde_json::to_vec(&loser.definition).map_err(|e| e.to_string())?
    {
        backup_conflict(document, loser)?;
    }
    let changed = document
        .records
        .get(&winner.key)
        .is_none_or(|r| r.revision != winner.revision);
    document.records.insert(winner.key.clone(), winner);
    // A backup-only merge still needs committing once, and is idempotent on repeated frames.
    Ok(changed || document.conflicts.len() != before_conflicts)
}
fn validate_record(record: &Record) -> Result<(), String> {
    valid_id(&record.revision.author)?;
    if record.revision.counter == 0
        || record.revision.counter > u64::MAX - 1024
        || record.clock.is_empty()
        || record.clock.len() > 128
        || record.clock.get(&record.revision.author) != Some(&record.revision.counter)
    {
        return Err("工作流修订信息无效".into());
    }
    for (author, counter) in &record.clock {
        valid_id(author)?;
        if *counter == 0 || *counter > record.revision.counter {
            return Err("工作流因果修订信息无效".into());
        }
    }
    let (kind, id) = record.key.split_once(':').ok_or("工作流记录标识无效")?;
    valid_id(id)?;
    if !matches!(kind, "workflow" | "rule") {
        return Err("工作流记录类型无效".into());
    }
    match &record.definition {
        Some(Definition::Workflow(workflow)) => {
            if kind != "workflow" || workflow.id != id || workflow.version != 2 {
                return Err("工作流定义标识不一致".into());
            }
            workflow.validate()?;
            let mut tokens = BTreeSet::new();
            for step in &workflow.steps {
                if step.shortcut_name.len() > 1024 {
                    return Err("快捷指令名称过长".into());
                }
                if step.kind == WorkflowStepKind::SystemShortcut
                    && (!step.shortcut_id.starts_with(TOKEN_PREFIX)
                        || !tokens.insert(&step.shortcut_id))
                {
                    return Err("工作流同步不得包含本机快捷指令标识".into());
                }
            }
        }
        Some(Definition::Rule(rule)) => {
            if kind != "rule" || rule.id != id {
                return Err("共享规则标识不一致".into());
            }
            rule.validate()?;
        }
        None => {}
    }
    encode(&Message::Record {
        record: record.clone(),
    })?;
    Ok(())
}
fn validate_document(document: &Document) -> Result<(), String> {
    valid_id(&document.local_device_id)?;
    if document.records.len() > MAX_RECORDS
        || document.conflicts.len() > MAX_CONFLICTS
        || document.bindings.len() > MAX_RECORDS * 32
    {
        return Err("工作流目录超过容量限制".into());
    }
    for (key, record) in &document.records {
        if key != &record.key {
            return Err("工作流目录索引损坏".into());
        }
        validate_record(record)?;
    }
    for conflict in document.conflicts.values() {
        validate_record(&conflict.record)?;
    }
    Ok(())
}
