use super::*;
use axum::{Json, extract::Multipart, http::HeaderMap};
use deskfuse_platform::{PasteboardItem, PasteboardRepresentation, PasteboardSnapshot};
use deskfuse_transfer::TransferId;
use tokio::io::AsyncWriteExt;

#[derive(Deserialize)]
pub(super) struct TextRequest {
    text: String,
    request_id: String,
}
struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
struct Allocation {
    bridge: Arc<TokioBridge>,
    id: TransferId,
    root: PathBuf,
    keep: bool,
}
impl Drop for Allocation {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.root);
        }
        let _ = self.bridge.workflow_cache_finish(self.id);
    }
}
struct Upload {
    shared: Arc<Shared>,
    id: String,
    cancel: Arc<AtomicBool>,
    cache: Option<Allocation>,
    finished: bool,
}
impl Drop for Upload {
    fn drop(&mut self) {
        if !self.finished {
            self.shared.update_task(&self.id, |t| {
                t.state = "cancelled".into();
                t.message = "接收已取消，请重试".into();
            });
        }
    }
}
fn begin(
    s: &HttpState,
    id: String,
    name: String,
    total: Option<u64>,
    cancel: Arc<AtomicBool>,
) -> Result<std::result::Result<Upload, Value>> {
    uuid::Uuid::parse_str(&id).map_err(|_| "上传请求标识无效")?;
    let mut i = s.shared.inner.lock().map_err(|_| "任务状态不可用")?;
    if let Some(t) = i.tasks.iter().find(|t| t.id == id) {
        if let Some(result) = &t.result {
            return Ok(Err(result.clone()));
        }
        if matches!(t.state.as_str(), "receiving" | "committing") {
            return Err("此上传正在处理，请稍候".into());
        }
    }
    i.tasks.retain(|t| t.id != id);
    while i.tasks.len() >= 100 {
        let Some(index) = i
            .tasks
            .iter()
            .rposition(|t| !matches!(t.state.as_str(), "receiving" | "committing"))
        else {
            return Err("接收任务过多，请稍候".into());
        };
        i.tasks.remove(index);
    }
    i.tasks.push_front(UploadTask {
        id: id.clone(),
        name,
        state: "receiving".into(),
        received_bytes: 0,
        total_bytes: total,
        clipboard_written: None,
        message: "正在接收".into(),
        result: None,
    });
    Ok(Ok(Upload {
        shared: s.shared.clone(),
        id,
        cancel,
        cache: None,
        finished: false,
    }))
}
fn fail(upload: &mut Upload, message: &str) {
    upload.finished = true;
    upload.shared.update_task(&upload.id, |t| {
        t.state = "error".into();
        t.message = message.into();
    });
}
fn commit(
    mut upload: Upload,
    bridge: Arc<TokioBridge>,
    snapshot: PasteboardSnapshot,
    items: Vec<PasteboardItem>,
    baseline: i64,
) -> Result<Value> {
    upload.shared.update_task(&upload.id, |t| {
        t.state = "committing".into();
        t.message = "正在加入列表".into();
    });
    let outcome = bridge.lan_web_commit(snapshot, items, baseline, upload.cancel.clone());
    match outcome {
        Ok(result) => {
            if let Some(cache) = &mut upload.cache {
                cache.keep = true;
            }
            upload.finished = true;
            let value = json!({"task_id":upload.id,"item_id":result.item_id.to_string(),"clipboard_written":result.clipboard_written,"message":result.message});
            upload.shared.update_task(&upload.id, |t| {
                t.state = "done".into();
                t.clipboard_written = Some(result.clipboard_written);
                t.message = result.message;
                t.result = Some(value.clone());
            });
            Ok(value)
        }
        Err(e) => {
            fail(&mut upload, &e);
            Err(e)
        }
    }
}
pub(super) async fn text(State(s): State<HttpState>, Json(request): Json<TextRequest>) -> Response {
    if request.text.trim().is_empty() {
        return error(StatusCode::BAD_REQUEST, "请输入要发送的文字");
    }
    let bridge = match s.shared.bridge() {
        Ok(b) => b,
        Err(e) => return error(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    let flag = Arc::new(AtomicBool::new(false));
    let _cancel = CancelOnDrop(flag.clone());
    let mut upload = match begin(
        &s,
        request.request_id,
        "文字".into(),
        Some(request.text.len() as u64),
        flag,
    ) {
        Ok(Ok(u)) => u,
        Ok(Err(v)) => return Json(v).into_response(),
        Err(e) => return error(StatusCode::CONFLICT, e),
    };
    let b = bridge.clone();
    let baseline = match blocking(move || b.lan_web_context()).await {
        Ok(v) => v.change_count,
        Err(e) => {
            fail(&mut upload, &e);
            return error(StatusCode::SERVICE_UNAVAILABLE, e);
        }
    };
    let permit = match s.shared.uploads.clone().acquire_owned().await {
        Ok(p) => p,
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "服务已关闭"),
    };
    upload
        .shared
        .update_task(&upload.id, |t| t.received_bytes = request.text.len() as u64);
    match blocking(move || {
        let _permit = permit;
        commit(
            upload,
            bridge,
            PasteboardSnapshot {
                text: Some(request.text),
                ..Default::default()
            },
            vec![],
            baseline,
        )
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, e),
    }
}
fn photo_name(raw: &str) -> Result<String> {
    let name = raw.rsplit(['/', '\\']).next().unwrap_or("").trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 240
        || name.chars().any(|c| c.is_control())
    {
        return Err("照片文件名无效".into());
    }
    if super::content::image_kind(name).is_none() {
        return Err("请选择 JPEG、PNG、HEIC、TIFF、GIF 或 WebP 图片".into());
    }
    Ok(name.into())
}
pub(super) async fn photo(
    State(s): State<HttpState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let total = match headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        Some(n) if n > 0 && n <= MAX_PHOTO_BODY as u64 => n,
        _ => {
            return error(
                StatusCode::LENGTH_REQUIRED,
                "照片大小未知或超过 256 MiB，请重新选择",
            );
        }
    };
    // The ID precedes photo bytes, allowing completed retries to return without receiving again.
    let mut first = match multipart.next_field().await {
        Ok(Some(f)) if f.name() == Some("request_id") => f,
        _ => return error(StatusCode::BAD_REQUEST, "缺少上传请求标识"),
    };
    let mut id_bytes = Vec::new();
    loop {
        match first.chunk().await {
            Ok(Some(bytes)) if id_bytes.len() + bytes.len() <= 64 => {
                id_bytes.extend_from_slice(&bytes)
            }
            Ok(None) => break,
            _ => return error(StatusCode::BAD_REQUEST, "上传请求标识无效"),
        }
    }
    drop(first);
    let id = match String::from_utf8(id_bytes) {
        Ok(id) => id,
        Err(_) => return error(StatusCode::BAD_REQUEST, "上传请求标识无效"),
    };
    let mut field = match multipart.next_field().await {
        Ok(Some(f)) if f.name() == Some("photo") => f,
        _ => return error(StatusCode::BAD_REQUEST, "请选择照片"),
    };
    let name = match photo_name(field.file_name().unwrap_or("")) {
        Ok(n) => n,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    let bridge = match s.shared.bridge() {
        Ok(b) => b,
        Err(e) => return error(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    let flag = Arc::new(AtomicBool::new(false));
    let _cancel = CancelOnDrop(flag.clone());
    let mut upload = match begin(&s, id, name.clone(), Some(total), flag.clone()) {
        Ok(Ok(u)) => u,
        Ok(Err(v)) => return Json(v).into_response(),
        Err(e) => return error(StatusCode::CONFLICT, e),
    };
    let b = bridge.clone();
    let baseline = match blocking(move || b.lan_web_context()).await {
        Ok(v) => v.change_count,
        Err(e) => {
            fail(&mut upload, &e);
            return error(StatusCode::SERVICE_UNAVAILABLE, e);
        }
    };
    let permit = match s.shared.uploads.clone().acquire_owned().await {
        Ok(p) => p,
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "服务已关闭"),
    };
    let cache_id = TransferId::new();
    let b = bridge.clone();
    // Reserve the declared complete body before writing any bytes. Multipart overhead is
    // deliberately included; no unbounded chunked transfer can escape the shared quota.
    let reservation = blocking(move || {
        let root = b.workflow_cache_reserve(cache_id, total)?;
        Ok(Allocation {
            bridge: b,
            id: cache_id,
            root,
            keep: false,
        })
    })
    .await;
    let allocation = match reservation {
        Ok(v) => v,
        Err(e) => {
            fail(&mut upload, &e);
            return error(StatusCode::INSUFFICIENT_STORAGE, e);
        }
    };
    let root = allocation.root.clone();
    upload.cache = Some(allocation);
    let path = root.join(&name);
    let receive = async {
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(|e| e.to_string())?;
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
            .map_err(|e| e.to_string())?;
        let mut received = 0u64;
        while let Some(bytes) = tokio::time::timeout(Duration::from_secs(60), field.chunk())
            .await
            .map_err(|_| "接收照片超时")?
            .map_err(|_| "照片上传中断，请重试")?
        {
            received = received.checked_add(bytes.len() as u64).ok_or("照片过大")?;
            if received > total || received > 256 * 1024 * 1024 {
                return Err("照片超过大小限制".into());
            }
            file.write_all(&bytes).await.map_err(|e| e.to_string())?;
            upload
                .shared
                .update_task(&upload.id, |t| t.received_bytes = received);
        }
        if received == 0 {
            return Err("照片为空".into());
        }
        file.flush().await.map_err(|e| e.to_string())?;
        file.sync_all().await.map_err(|e| e.to_string())?;
        upload.shared.update_task(&upload.id, |t| {
            t.total_bytes = Some(received);
            t.received_bytes = received;
        });
        Ok::<(), String>(())
    }
    .await;
    drop(field);
    if let Err(e) = receive {
        fail(&mut upload, &e);
        return error(StatusCode::BAD_REQUEST, e);
    }
    match multipart.next_field().await {
        Ok(None) => {}
        _ => {
            let message = "每个请求只能上传一张照片，且数据必须完整";
            fail(&mut upload, message);
            return error(StatusCode::BAD_REQUEST, message);
        }
    }
    match blocking(move || {
        let _permit = permit;
        if flag.load(Ordering::Acquire) {
            return Err("上传已取消".into());
        }
        let string = path.to_string_lossy().into_owned();
        // Original image representations are for the pasteboard only. The library keeps
        // the original file reference, never a second copy of image bytes in SQLite.
        let mut representations = vec![PasteboardRepresentation {
            type_name: "public.file-url".into(),
            data: url::Url::from_file_path(&path)
                .map_err(|_| "照片路径无效")?
                .to_string()
                .into_bytes(),
        }];
        let preview_ok = deskfuse_platform_macos::lan_web::image_png(&path, 320).is_ok();
        if preview_ok
            && let (Some(kind), Ok(data)) =
                (super::content::image_kind(&string), std::fs::read(&path))
        {
            representations.push(PasteboardRepresentation {
                type_name: kind.into(),
                data,
            });
        }
        let kind = super::content::image_kind(&string).unwrap_or("public.jpeg");
        let mime = match kind {
            "public.png" => "image/png",
            "public.heic" => "image/heic",
            "public.tiff" => "image/tiff",
            "com.compuserve.gif" => "image/gif",
            "org.webmproject.webp" => "image/webp",
            _ => "image/jpeg",
        };
        let metadata =
            json!({"mime":mime,"size":std::fs::metadata(&path).map_err(|e|e.to_string())?.len()});
        let snapshot = PasteboardSnapshot {
            file_urls: vec![string],
            items: vec![PasteboardItem {
                representations: vec![PasteboardRepresentation {
                    type_name: crate::clipboard_bundle::LAN_PHOTO_TYPE.into(),
                    data: serde_json::to_vec(&metadata).map_err(|e| e.to_string())?,
                }],
            }],
            ..Default::default()
        };
        let shared = upload.shared.clone();
        let task_id = upload.id.clone();
        let mut result = commit(
            upload,
            bridge,
            snapshot,
            vec![PasteboardItem { representations }],
            baseline,
        )?;
        if !preview_ok {
            let base = result["message"].as_str().unwrap_or("").to_string();
            let message = format!("{base}；原文件已保留，图片预览暂不可用");
            result["message"] = json!(message);
            shared.update_task(&task_id, |t| {
                t.message = message;
                t.result = Some(result.clone());
            });
        }
        Ok(result)
    })
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, e),
    }
}
