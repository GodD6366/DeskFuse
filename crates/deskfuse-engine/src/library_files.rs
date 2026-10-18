//! Stable file archives: source paths stay in local SQLite; only opaque references
//! cross devices. Explicit materialization uses the existing secure file receiver
//! and never writes the system pasteboard. Caller owns cache reservation/pinning.
use deskfuse_core::{ClipboardPayload, DeviceId};
use deskfuse_platform::{PasteboardItem, PasteboardRepresentation, PasteboardSnapshot};
use deskfuse_runtime::RuntimeHandle;
use deskfuse_transfer::{
    ClipboardFileFrame, ClipboardFilePlan, ConflictStrategy, FileReceiver, TransferId,
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub const MIME: &str = "application/x-deskfuse-library-files-v1";
pub const REFERENCE_TYPE: &str = "com.deskfuse.library-file-reference-v1";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reference {
    pub resource_id: String,
    pub device_id: String,
    pub manifest_hash: String,
    pub names: Vec<String>,
    pub total_bytes: u64,
}
fn db() -> Result<Connection> {
    let mut db =
        Connection::open(deskfuse_store::SQLiteStore::default_path().map_err(err)?).map_err(err)?;
    deskfuse_store::configure_connection(&mut db).map_err(err)?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS cl_file_resources(id TEXT PRIMARY KEY,paths TEXT NOT NULL,manifest_hash TEXT NOT NULL)").map_err(err)?;
    Ok(db)
}
pub fn reference(snapshot: &PasteboardSnapshot) -> Result<Option<Reference>> {
    let rep = snapshot
        .items
        .iter()
        .flat_map(|i| &i.representations)
        .find(|r| r.type_name == REFERENCE_TYPE);
    rep.map(|r| serde_json::from_slice(&r.data).map_err(err))
        .transpose()
}
/// Metadata-only availability probe. Download/restore still uses resolve_local,
/// which validates the complete resource manifest before exporting bytes.
pub(crate) fn local_paths_for_preview(
    snapshot: &PasteboardSnapshot,
    local_device: &str,
) -> Result<Option<Vec<String>>> {
    let Some(reference) = reference(snapshot)? else {
        return Ok(Some(snapshot.file_urls.clone()));
    };
    if reference.device_id != local_device {
        return Ok(None);
    }
    let raw: String = db()?
        .query_row(
            "SELECT paths FROM cl_file_resources WHERE id=?1 AND manifest_hash=?2",
            params![reference.resource_id, reference.manifest_hash],
            |row| row.get(0),
        )
        .map_err(err)?;
    serde_json::from_str(&raw).map(Some).map_err(err)
}

/// Called on a blocking worker when first adding local files to a manual list.
/// Hashes the complete manifest; returns a path-free, transmissible snapshot.
pub fn register(snapshot: &PasteboardSnapshot, local_device: &str) -> Result<PasteboardSnapshot> {
    if let Some(reference) = reference(snapshot)? {
        return portable(snapshot, &reference);
    }
    if snapshot.file_urls.is_empty() {
        return Ok(snapshot.clone());
    }
    DeviceId::from_hex(local_device).map_err(err)?;
    let paths = snapshot
        .file_urls
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let plan = ClipboardFilePlan::from_paths(&paths).map_err(err)?;
    let resource_id = uuid::Uuid::new_v4().to_string();
    let manifest_hash = plan.manifest().digest().to_hex();
    db()?
        .execute(
            "INSERT INTO cl_file_resources VALUES(?1,?2,?3)",
            params![
                resource_id,
                serde_json::to_string(&snapshot.file_urls).map_err(err)?,
                manifest_hash
            ],
        )
        .map_err(err)?;
    let names = plan
        .manifest()
        .entries()
        .iter()
        .filter_map(|e| e.path().as_str().split('/').next())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let reference = Reference {
        resource_id,
        device_id: local_device.into(),
        manifest_hash,
        names,
        total_bytes: plan.manifest().total_bytes(),
    };
    portable(snapshot, &reference)
}
fn portable(snapshot: &PasteboardSnapshot, reference: &Reference) -> Result<PasteboardSnapshot> {
    let mut result = snapshot.clone();
    result.file_urls.clear();
    result.text = None;
    result.png = None;
    // File aliases, bookmarks and arbitrary Finder representations may contain
    // absolute source paths. Keep only the engine's source-policy metadata.
    let mut reps = result
        .items
        .iter()
        .flat_map(|i| &i.representations)
        .filter(|r| r.type_name == crate::clipboard_bundle::SOURCE_METADATA_TYPE)
        .cloned()
        .collect::<Vec<_>>();
    reps.push(PasteboardRepresentation {
        type_name: REFERENCE_TYPE.into(),
        data: serde_json::to_vec(reference).map_err(err)?,
    });
    result.items = vec![PasteboardItem {
        representations: reps,
    }];
    Ok(result)
}
fn source_plan(reference: &Reference, id: TransferId) -> Result<(ClipboardFilePlan, Vec<String>)> {
    let (raw, expected): (String, String) = db()?
        .query_row(
            "SELECT paths,manifest_hash FROM cl_file_resources WHERE id=?1",
            [&reference.resource_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "来源文件引用已失效".to_owned())?;
    if expected != reference.manifest_hash {
        return Err("来源资源身份不匹配".into());
    }
    let paths: Vec<String> = serde_json::from_str(&raw).map_err(err)?;
    let plan = ClipboardFilePlan::from_paths_with_transfer_id(
        &paths.iter().map(PathBuf::from).collect::<Vec<_>>(),
        id,
    )
    .map_err(|_| "来源文件已删除、移动或无法读取".to_owned())?;
    if plan.manifest().digest().to_hex() != expected
        || plan.manifest().total_bytes() != reference.total_bytes
    {
        return Err("来源文件或目录结构已变化，请重新归档".into());
    }
    Ok((plan, paths))
}
/// Existing local source references may be resolved without a network request,
/// but are still rehashed before use. Returns None for a remote reference.
pub fn resolve_local(
    snapshot: &PasteboardSnapshot,
    local_device: &str,
) -> Result<Option<PasteboardSnapshot>> {
    let Some(r) = reference(snapshot)? else {
        return Ok(Some(snapshot.clone()));
    };
    if r.device_id != local_device {
        return Ok(None);
    }
    let (_, paths) = source_plan(&r, TransferId::new())?;
    let mut out = snapshot.clone();
    out.file_urls = paths;
    Ok(Some(out))
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Frame {
    Request {
        request_id: String,
        reference: Reference,
    },
    Data {
        request_id: String,
        data: String,
    },
    Ack {
        request_id: String,
    },
    Error {
        request_id: String,
        message: String,
    },
}
impl Frame {
    fn id(&self) -> &str {
        match self {
            Self::Request { request_id, .. }
            | Self::Data { request_id, .. }
            | Self::Ack { request_id }
            | Self::Error { request_id, .. } => request_id,
        }
    }
}
type Key = (DeviceId, u64, String);
type Mailboxes = HashMap<Key, mpsc::Sender<Frame>>;
fn mailboxes() -> &'static Mutex<Mailboxes> {
    static BOXES: OnceLock<Mutex<Mailboxes>> = OnceLock::new();
    BOXES.get_or_init(|| Mutex::new(HashMap::new()))
}
struct Mailbox(Key);
impl Drop for Mailbox {
    fn drop(&mut self) {
        if let Ok(mut boxes) = mailboxes().lock() {
            boxes.remove(&self.0);
        }
    }
}
fn mailbox(peer: DeviceId, generation: u64, id: &str) -> Result<(Mailbox, mpsc::Receiver<Frame>)> {
    let key = (peer, generation, id.into());
    let mut boxes = mailboxes().lock().map_err(err)?;
    if boxes.contains_key(&key) || boxes.len() >= 16 {
        return Err("文件归档任务正在运行，请稍后重试".into());
    }
    let (tx, rx) = mpsc::channel(2);
    boxes.insert(key.clone(), tx);
    Ok((Mailbox(key), rx))
}
async fn send(
    runtime: &RuntimeHandle,
    peer: DeviceId,
    generation: u64,
    frame: Frame,
) -> Result<()> {
    runtime
        .send_clipboard_on_session(
            peer,
            generation,
            ClipboardPayload::new(MIME, serde_json::to_vec(&frame).map_err(err)?, 1)
                .map_err(err)?,
        )
        .await
        .map_err(err)
}
async fn next(
    rx: &mut mpsc::Receiver<Frame>,
    runtime: &RuntimeHandle,
    peer: DeviceId,
    generation: u64,
    cancel: &AtomicBool,
) -> Result<Frame> {
    let deadline = tokio::time::sleep(Duration::from_secs(30));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _=&mut deadline=>return Err("来源设备响应超时，请检查对端版本和连接".into()),
            value=rx.recv()=>return match value{Some(Frame::Error{message,..})=>Err(message),Some(v)=>Ok(v),None=>Err("文件请求已取消".into())},
            _=tokio::time::sleep(Duration::from_millis(100))=>{if cancel.load(Ordering::Acquire)||runtime.control_generation(peer)!=Some(generation){return Err("文件请求已取消或连接已断开".into())}}
        }
    }
}
/// Bridge must invoke this only for authenticated trusted_connected_peer_ids.
/// Non-request frames are accepted only for a locally registered session mailbox.
pub fn receive(peer: DeviceId, bytes: &[u8], runtime: &RuntimeHandle) {
    if bytes.len() > 3 * 1024 * 1024 {
        return;
    }
    let Ok(frame) = serde_json::from_slice::<Frame>(bytes) else {
        return;
    };
    let Some(generation) = runtime.control_generation(peer) else {
        return;
    };
    if uuid::Uuid::parse_str(frame.id()).is_err() {
        return;
    }
    if let Frame::Request {
        request_id,
        reference,
    } = frame
    {
        let Ok((guard, rx)) = mailbox(peer, generation, &request_id) else {
            return;
        };
        let runtime = runtime.clone();
        tokio::spawn(async move {
            let _guard = guard;
            let result = serve(&runtime, peer, generation, &request_id, reference, rx).await;
            if let Err(message) = result {
                let _ = send(
                    &runtime,
                    peer,
                    generation,
                    Frame::Error {
                        request_id,
                        message,
                    },
                )
                .await;
            }
        });
    } else if let Ok(boxes) = mailboxes().lock()
        && let Some(tx) = boxes.get(&(peer, generation, frame.id().into()))
    {
        let _ = tx.try_send(frame);
    }
}
async fn serve(
    runtime: &RuntimeHandle,
    peer: DeviceId,
    generation: u64,
    request_id: &str,
    reference: Reference,
    mut rx: mpsc::Receiver<Frame>,
) -> Result<()> {
    let id = TransferId::from_uuid(uuid::Uuid::parse_str(request_id).map_err(err)?);
    let (plan, _) = tokio::task::spawn_blocking(move || source_plan(&reference, id))
        .await
        .map_err(err)??;
    let cancel = AtomicBool::new(false);
    let offer = ClipboardFileFrame::Offer {
        transfer_id: id,
        manifest: plan.manifest().clone(),
    };
    send_piece(
        runtime, peer, generation, request_id, offer, &mut rx, &cancel,
    )
    .await?;
    for source in plan.sources() {
        for descriptor in source.chunks() {
            let path = source.source_path().to_owned();
            let descriptor = *descriptor;
            let bytes = tokio::task::spawn_blocking(move || {
                deskfuse_transfer::read_chunk(path, descriptor).map_err(err)
            })
            .await
            .map_err(err)??;
            let frame = ClipboardFileFrame::Chunk {
                transfer_id: id,
                path: source.relative_path().clone(),
                descriptor,
                bytes,
            };
            send_piece(
                runtime, peer, generation, request_id, frame, &mut rx, &cancel,
            )
            .await?;
        }
    }
    send_piece(
        runtime,
        peer,
        generation,
        request_id,
        ClipboardFileFrame::Complete { transfer_id: id },
        &mut rx,
        &cancel,
    )
    .await
}
async fn send_piece(
    runtime: &RuntimeHandle,
    peer: DeviceId,
    generation: u64,
    id: &str,
    piece: ClipboardFileFrame,
    rx: &mut mpsc::Receiver<Frame>,
    cancel: &AtomicBool,
) -> Result<()> {
    send(
        runtime,
        peer,
        generation,
        Frame::Data {
            request_id: id.into(),
            data: hex::encode(piece.encode().map_err(err)?),
        },
    )
    .await?;
    match next(rx, runtime, peer, generation, cancel).await? {
        Frame::Ack { .. } => Ok(()),
        _ => Err("文件传输确认无效".into()),
    }
}
/// Reserve `reference(snapshot).total_bytes` for `transfer_id` in the shared
/// TransferCache BEFORE calling; destination must be cache.destination(id).
/// Keep that reservation/pin until restore has installed the returned URLs.
/// This future prepares files only, never writes the system pasteboard.
pub async fn materialize(
    snapshot: PasteboardSnapshot,
    transfer_id: TransferId,
    destination: PathBuf,
    runtime: RuntimeHandle,
    cancel: Arc<AtomicBool>,
) -> Result<PasteboardSnapshot> {
    let r = reference(&snapshot)?.ok_or("不是文件归档")?;
    let peer = DeviceId::from_hex(&r.device_id).map_err(err)?;
    let generation = runtime
        .control_generation(peer)
        .ok_or("来源设备离线，请连接后重试")?;
    let id = transfer_id.as_uuid().to_string();
    let (_guard, mut rx) = mailbox(peer, generation, &id)?;
    send(
        &runtime,
        peer,
        generation,
        Frame::Request {
            request_id: id.clone(),
            reference: r.clone(),
        },
    )
    .await?;
    // All writes/hashes run in one blocking worker with bounded network handoff.
    let (tx, mut disk_rx) =
        mpsc::channel::<(ClipboardFileFrame, tokio::sync::oneshot::Sender<Result<()>>)>(1);
    let disk_destination = destination.clone();
    let disk_cancel = cancel.clone();
    let expected = r.clone();
    let disk = tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
        let result = (|| {
            if disk_cancel.load(Ordering::Acquire) {
                return Err("文件请求已取消".into());
            }
            if let Some(parent) = disk_destination.parent() {
                std::fs::create_dir_all(parent).map_err(err)?
            }
            let receiver = FileReceiver::new(&disk_destination).map_err(err)?;
            let mut receiver = Some(receiver);
            let mut staging = None;
            let mut names = Vec::new();
            while let Some((piece, reply)) = disk_rx.blocking_recv() {
                if disk_cancel.load(Ordering::Acquire) {
                    let _ = reply.send(Err("文件请求已取消".into()));
                    return Err("文件请求已取消".into());
                }
                let complete = matches!(&piece, ClipboardFileFrame::Complete { .. });
                let step = (|| -> Result<()> {
                    match piece {
                        ClipboardFileFrame::Offer { manifest, .. } if staging.is_none() => {
                            if manifest.digest().to_hex() != expected.manifest_hash
                                || manifest.total_bytes() != expected.total_bytes
                            {
                                return Err("文件归档清单与引用不一致".into());
                            }
                            names = manifest
                                .entries()
                                .iter()
                                .filter_map(|e| e.path().as_str().split('/').next())
                                .map(str::to_string)
                                .collect::<BTreeSet<_>>()
                                .into_iter()
                                .collect();
                            staging = Some(
                                receiver
                                    .as_ref()
                                    .ok_or("接收器已关闭")?
                                    .begin(transfer_id, &manifest)
                                    .map_err(err)?,
                            );
                            Ok(())
                        }
                        ClipboardFileFrame::Chunk {
                            path,
                            descriptor,
                            bytes,
                            ..
                        } => staging
                            .as_mut()
                            .ok_or("缺少文件清单")?
                            .write_chunk(&path, descriptor, &bytes)
                            .map_err(err),
                        ClipboardFileFrame::Complete { .. } => {
                            receiver
                                .take()
                                .ok_or("接收器已关闭")?
                                .commit_staging(
                                    staging.take().ok_or("缺少文件清单")?,
                                    ConflictStrategy::Overwrite,
                                )
                                .map_err(err)?;
                            Ok(())
                        }
                        _ => Err("文件归档帧顺序无效".into()),
                    }
                })();
                let _ = reply.send(step.clone());
                step?;
                if complete {
                    return Ok(names
                        .iter()
                        .map(|n| disk_destination.join(n).to_string_lossy().into_owned())
                        .collect());
                }
            }
            Err("文件请求已取消".into())
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&disk_destination);
        }
        result
    });
    let result: Result<()> = async {
        loop {
            let frame = next(&mut rx, &runtime, peer, generation, &cancel).await?;
            let Frame::Data { data, .. } = frame else {
                return Err("无效文件归档响应".into());
            };
            let piece =
                ClipboardFileFrame::decode(&hex::decode(data).map_err(err)?).map_err(err)?;
            if piece.transfer_id() != transfer_id {
                return Err("文件请求身份不一致".into());
            }
            let complete = matches!(&piece, ClipboardFileFrame::Complete { .. });
            let (done, result) = tokio::sync::oneshot::channel();
            tx.send((piece, done)).await.map_err(err)?;
            result.await.map_err(err)??;
            send(
                &runtime,
                peer,
                generation,
                Frame::Ack {
                    request_id: id.clone(),
                },
            )
            .await?;
            if complete {
                return Ok(());
            }
        }
    }
    .await;
    drop(tx);
    if let Err(message) = result {
        let _ = send(
            &runtime,
            peer,
            generation,
            Frame::Error {
                request_id: id,
                message: message.clone(),
            },
        )
        .await;
        let _ = disk.await;
        let _ = std::fs::remove_dir_all(&destination);
        return Err(message);
    }
    let paths = disk.await.map_err(err)??;
    if cancel.load(Ordering::Acquire) || runtime.control_generation(peer) != Some(generation) {
        let _ = std::fs::remove_dir_all(&destination);
        return Err("文件请求已取消或连接已断开".into());
    }
    let mut out = snapshot;
    out.file_urls = paths;
    Ok(out)
}
