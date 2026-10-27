//! LAN web uploads confirmed by the authenticated desktop clipboard session.

use crate::desktop_protocol as ipc;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine;
use qrcode::{Color, QrCode};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, VecDeque},
    io::Write,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::Notify, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

pub const LAN_WEB_PORT: u16 = 42426;
const MAX_TEXT_BODY: usize = 1024 * 1024;
const MAX_PHOTO_BODY: usize = 16 * 1024 * 1024 + 64 * 1024;
const MAX_CONTENT: usize = 16 * 1024 * 1024;
const MAX_PIXELS: u64 = 24_000_000;
const MAX_PENDING: usize = 4;

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct Settings {
    enabled: bool,
}

struct PendingUpload {
    upload: ipc::LanWebUpload,
    delivered: bool,
}

struct Inner {
    settings: Settings,
    epoch: u64,
    running: bool,
    session_available: bool,
    port: u16,
    address: Option<String>,
    addresses: Vec<String>,
    local_ips: BTreeSet<String>,
    error: Option<String>,
    tasks: VecDeque<ipc::LanWebTask>,
    pending: VecDeque<PendingUpload>,
    qr_size: usize,
    qr_modules: Vec<bool>,
}

struct Shared {
    inner: Mutex<Inner>,
    path: PathBuf,
    local_name: String,
    networks: Arc<dyn Fn() -> Vec<Ipv4Addr> + Send + Sync>,
    bind_ip: String,
    configured_port: u16,
    changed: Notify,
    stop: CancellationToken,
    upload_slots: Arc<tokio::sync::Semaphore>,
}

#[derive(Clone)]
struct HttpState {
    shared: Arc<Shared>,
    bind_ip: String,
    port: u16,
    epoch: u64,
    cancel: CancellationToken,
}

struct LiveServer {
    epoch: u64,
    cancel: CancellationToken,
    handle: JoinHandle<()>,
}

#[derive(Clone)]
pub struct LanWebService {
    shared: Arc<Shared>,
    worker: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl LanWebService {
    pub fn new(path: PathBuf, local_name: String) -> Self {
        Self::with_options(
            path,
            local_name,
            "0.0.0.0".into(),
            LAN_WEB_PORT,
            Arc::new(|| {
                deskfuse_platform_windows::lan_ipv4_networks()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(address, _)| address)
                    .collect()
            }),
        )
    }

    fn with_options(
        path: PathBuf,
        local_name: String,
        bind_ip: String,
        configured_port: u16,
        networks: Arc<dyn Fn() -> Vec<Ipv4Addr> + Send + Sync>,
    ) -> Self {
        let (settings, error) = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(settings) => (settings, None),
                Err(_) => (
                    Settings::default(),
                    Some("局域网设置读取失败，请重新开启服务".into()),
                ),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (Settings::default(), None)
            }
            Err(_) => (Settings::default(), Some("局域网设置无法读取".into())),
        };
        let shared = Arc::new(Shared {
            inner: Mutex::new(Inner {
                settings,
                epoch: 0,
                running: false,
                session_available: false,
                port: configured_port,
                address: None,
                addresses: Vec::new(),
                local_ips: BTreeSet::new(),
                error,
                tasks: VecDeque::new(),
                pending: VecDeque::new(),
                qr_size: 0,
                qr_modules: Vec::new(),
            }),
            path,
            local_name,
            networks,
            bind_ip,
            configured_port,
            changed: Notify::new(),
            stop: CancellationToken::new(),
            upload_slots: Arc::new(tokio::sync::Semaphore::new(2)),
        });
        let worker = tokio::spawn(serve(shared.clone()));
        Self {
            shared,
            worker: Arc::new(Mutex::new(Some(worker))),
        }
    }

    pub fn state(&self) -> ipc::LanWebState {
        let inner = self.shared.inner.lock().unwrap_or_else(|e| e.into_inner());
        ipc::LanWebState {
            enabled: inner.settings.enabled,
            running: inner.running,
            session_available: inner.session_available,
            address: inner.address.clone(),
            addresses: inner.addresses.clone(),
            port: inner.port,
            error: inner.error.clone(),
            tasks: inner.tasks.iter().cloned().collect(),
            qr_size: inner.qr_size,
            qr_modules: inner.qr_modules.clone(),
        }
    }

    pub fn configure(&self, enabled: bool) -> Result<ipc::LanWebState, String> {
        if self.shared.stop.is_cancelled() {
            return Err("服务已停止".into());
        }
        {
            let mut inner = self.shared.inner.lock().map_err(|_| "网页服务状态不可用")?;
            inner.settings.enabled = enabled;
            inner.epoch = inner.epoch.wrapping_add(1);
            inner.running = false;
            inner.address = None;
            inner.addresses.clear();
            inner.qr_size = 0;
            inner.qr_modules.clear();
            inner.error = None;
            if !enabled {
                cancel_pending(&mut inner, "局域网共享已关闭");
            }
        }
        self.shared.changed.notify_one();
        if let Err(error) = save_settings(&self.shared.path, &Settings { enabled }) {
            if enabled {
                let mut inner = self.shared.inner.lock().map_err(|_| "网页服务状态不可用")?;
                inner.settings.enabled = false;
                inner.epoch = inner.epoch.wrapping_add(1);
                inner.error = Some(error.clone());
                self.shared.changed.notify_one();
            }
            return Err(error);
        }
        Ok(self.state())
    }

    pub fn set_files_enabled(&self, enabled: bool) -> Result<ipc::LanWebState, String> {
        if enabled {
            Ok(self.state())
        } else {
            self.configure(false)
        }
    }

    pub fn resume_session(&self) {
        let mut inner = self.shared.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.session_available = true;
        drop(inner);
        self.shared.changed.notify_one();
    }

    pub fn pause_session(&self) {
        let mut inner = self.shared.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.session_available = false;
        inner.running = false;
        inner.address = None;
        inner.addresses.clear();
        inner.qr_size = 0;
        inner.qr_modules.clear();
        cancel_pending(&mut inner, "桌面会话已结束");
        drop(inner);
        self.shared.changed.notify_one();
    }

    pub fn deliveries(&self) -> Vec<ipc::LanWebUpload> {
        let mut inner = self.shared.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Inner { pending, tasks, .. } = &mut *inner;
        pending
            .iter_mut()
            .filter_map(|pending| {
                if pending.delivered {
                    return None;
                }
                pending.delivered = true;
                if let Some(task) = inner_task_mut(tasks, &pending.upload.id) {
                    task.state = "delivered".into();
                    task.message = "等待电脑确认".into();
                }
                Some(pending.upload.clone())
            })
            .collect()
    }

    pub fn commit(&self, id: &str, recorded: bool, clipboard_written: bool) -> bool {
        let mut inner = self.shared.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(task) = inner_task_mut(&mut inner.tasks, id) else {
            return false;
        };
        if !matches!(task.state.as_str(), "waiting" | "delivered") {
            return false;
        }
        task.recorded = Some(recorded);
        task.clipboard_written = Some(clipboard_written);
        task.state = if recorded {
            "done".into()
        } else {
            "error".into()
        };
        task.message = match (recorded, clipboard_written) {
            (true, true) => "已接收并写入本机剪贴板。".into(),
            (true, false) => "内容已接收，未覆盖当前剪贴板。".into(),
            (false, _) => "桌面未能接收内容，请重试。".into(),
        };
        task.result = Some(task_result(task));
        inner.pending.retain(|pending| pending.upload.id != id);
        true
    }
}

impl Drop for LanWebService {
    fn drop(&mut self) {
        if Arc::strong_count(&self.worker) != 1 {
            return;
        }
        self.shared.stop.cancel();
        if let Ok(mut worker) = self.worker.lock()
            && let Some(worker) = worker.take()
        {
            worker.abort();
        }
    }
}

fn inner_task_mut<'a>(
    tasks: &'a mut VecDeque<ipc::LanWebTask>,
    id: &str,
) -> Option<&'a mut ipc::LanWebTask> {
    tasks.iter_mut().find(|task| task.id == id)
}

fn task_result(task: &ipc::LanWebTask) -> Value {
    json!({
        "task_id": task.id,
        "pending": false,
        "recorded": task.recorded.unwrap_or(false),
        "clipboard_written": task.clipboard_written.unwrap_or(false),
        "message": task.message,
    })
}

fn pending_result(id: &str) -> Value {
    json!({"task_id":id,"pending":true,"message":"已接收，等待电脑确认"})
}

fn cancel_pending(inner: &mut Inner, message: &str) {
    for task in &mut inner.tasks {
        if matches!(task.state.as_str(), "waiting" | "delivered") {
            task.state = "error".into();
            task.recorded = Some(false);
            task.clipboard_written = Some(false);
            task.message = message.into();
            task.result = Some(task_result(task));
        }
    }
    inner.pending.clear();
}

fn save_settings(path: &Path, settings: &Settings) -> Result<(), String> {
    let parent = path.parent().ok_or("局域网设置路径无效")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(".lan-web-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec(settings).map_err(|e| e.to_string())?)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        replace_file(&temporary, path)
    })();
    let _ = std::fs::remove_file(temporary);
    result.map_err(|error| format!("无法保存局域网设置：{error}"))
}

fn replace_file(source: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    let source = source
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    // SAFETY: both paths are NUL-terminated and remain alive for the call.
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

async fn serve(shared: Arc<Shared>) {
    let mut live: Option<LiveServer> = None;
    loop {
        let (enabled, epoch) = shared
            .inner
            .lock()
            .map(|inner| {
                (
                    inner.settings.enabled && inner.session_available,
                    inner.epoch,
                )
            })
            .unwrap_or((false, 0));
        let restart = shared.stop.is_cancelled()
            || !enabled && live.is_some()
            || live
                .as_ref()
                .is_some_and(|server| server.epoch != epoch || server.handle.is_finished());
        if restart {
            if let Some(server) = live.take() {
                server.cancel.cancel();
                server.handle.abort();
                let _ = server.handle.await;
            }
            let mut inner = shared.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.running = false;
            inner.address = None;
            inner.addresses.clear();
            inner.qr_size = 0;
            inner.qr_modules.clear();
        }
        if shared.stop.is_cancelled() {
            break;
        }
        if live.is_none() && enabled {
            match tokio::net::TcpListener::bind((shared.bind_ip.as_str(), shared.configured_port))
                .await
            {
                Ok(listener) => {
                    let Ok(local) = listener.local_addr() else {
                        continue;
                    };
                    let cancel = CancellationToken::new();
                    let state = HttpState {
                        shared: shared.clone(),
                        bind_ip: shared.bind_ip.clone(),
                        port: local.port(),
                        epoch,
                        cancel: cancel.clone(),
                    };
                    let app = router(state);
                    let stop = cancel.clone();
                    let handle = tokio::spawn(async move {
                        let _ = axum::serve(listener, app)
                            .with_graceful_shutdown(stop.cancelled_owned())
                            .await;
                    });
                    let mut inner = shared.inner.lock().unwrap_or_else(|e| e.into_inner());
                    if inner.epoch != epoch || !inner.settings.enabled || !inner.session_available {
                        cancel.cancel();
                        handle.abort();
                        continue;
                    }
                    inner.running = true;
                    inner.port = local.port();
                    inner.error = None;
                    live = Some(LiveServer {
                        epoch,
                        cancel,
                        handle,
                    });
                }
                Err(error) => {
                    let mut inner = shared.inner.lock().unwrap_or_else(|e| e.into_inner());
                    inner.error = Some(format!("无法监听局域网地址：{error}"));
                }
            }
        }
        if live.is_some() {
            let port = live.as_ref().map_or(0, |_| {
                shared.inner.lock().map(|inner| inner.port).unwrap_or(0)
            });
            let (addresses, local_ips) = addresses(&shared, port);
            let (size, modules) = addresses
                .first()
                .map(|address| qr_modules(address))
                .unwrap_or((0, Vec::new()));
            let mut inner = shared.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.address = addresses.first().cloned();
            inner.addresses = addresses;
            inner.local_ips = local_ips;
            inner.qr_size = size;
            inner.qr_modules = modules;
        }
        tokio::select! {
            _ = shared.stop.cancelled() => {},
            _ = shared.changed.notified() => {},
            _ = tokio::time::sleep(Duration::from_secs(1)) => {},
        }
    }
}

fn addresses(shared: &Shared, port: u16) -> (Vec<String>, BTreeSet<String>) {
    let mut local_ips = BTreeSet::from([Ipv4Addr::LOCALHOST.to_string()]);
    local_ips.extend((shared.networks)().into_iter().map(|ip| ip.to_string()));
    if shared.bind_ip != "0.0.0.0" {
        local_ips.retain(|ip| ip == &shared.bind_ip);
    }
    let addresses = local_ips
        .iter()
        .filter(|ip| {
            shared.bind_ip == Ipv4Addr::LOCALHOST.to_string()
                || *ip != &Ipv4Addr::LOCALHOST.to_string()
        })
        .map(|ip| {
            if port == 80 {
                format!("http://{ip}")
            } else {
                format!("http://{ip}:{port}")
            }
        })
        .collect::<Vec<_>>();
    (addresses, local_ips)
}

fn qr_modules(address: &str) -> (usize, Vec<bool>) {
    let Ok(code) = QrCode::new(address.as_bytes()) else {
        return (0, Vec::new());
    };
    (
        code.width(),
        code.to_colors()
            .into_iter()
            .map(|color| color == Color::Dark)
            .collect(),
    )
}

fn router(state: HttpState) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async {
                asset(
                    "text/html; charset=utf-8",
                    include_str!("lan_web_assets/index.html"),
                )
            }),
        )
        .route(
            "/app.css",
            get(|| async {
                asset(
                    "text/css; charset=utf-8",
                    include_str!("lan_web_assets/app.css"),
                )
            }),
        )
        .route(
            "/app.js",
            get(|| async {
                asset(
                    "text/javascript; charset=utf-8",
                    include_str!("lan_web_assets/app.js"),
                )
            }),
        )
        .route("/api/state", get(state_view))
        .route("/api/tasks", get(tasks))
        .route(
            "/api/text",
            post(upload_text).layer(DefaultBodyLimit::max(MAX_TEXT_BODY)),
        )
        .route(
            "/api/photos",
            post(upload_photo).layer(DefaultBodyLimit::max(MAX_PHOTO_BODY)),
        )
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

fn asset(mime: &'static str, content: &'static str) -> Response {
    ([(header::CONTENT_TYPE, mime)], content).into_response()
}

async fn state_view(State(state): State<HttpState>) -> Response {
    Json(json!({"name":state.shared.local_name})).into_response()
}

async fn tasks(State(state): State<HttpState>) -> Response {
    Json(json!({"tasks":state.tasks()})).into_response()
}

impl HttpState {
    fn tasks(&self) -> Vec<ipc::LanWebTask> {
        self.shared
            .inner
            .lock()
            .map(|inner| inner.tasks.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn active(&self) -> bool {
        self.shared
            .inner
            .lock()
            .map(|inner| {
                !self.shared.stop.is_cancelled()
                    && inner.settings.enabled
                    && inner.running
                    && inner.session_available
                    && inner.epoch == self.epoch
            })
            .unwrap_or(false)
    }
}

async fn guard(State(state): State<HttpState>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    let authority = host.and_then(|host| host.parse::<axum::http::uri::Authority>().ok());
    let allowed_host = authority.as_ref().is_some_and(|authority| {
        authority.port_u16().unwrap_or(80) == state.port
            && authority.host().parse::<Ipv4Addr>().is_ok()
            && if state.bind_ip == "0.0.0.0" {
                state
                    .shared
                    .inner
                    .lock()
                    .map(|inner| inner.local_ips.contains(authority.host()))
                    .unwrap_or(false)
            } else {
                authority.host() == state.bind_ip
            }
    });
    let expected = format!("http://{}", host.unwrap_or_default());
    if !allowed_host
        || origin.is_some_and(|origin| origin != expected)
        || request
            .headers()
            .get("sec-fetch-site")
            .is_some_and(|value| value == "cross-site")
    {
        return error(StatusCode::FORBIDDEN, "请从电脑显示的局域网地址进入");
    }
    if !state.active() {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "局域网服务已关闭或地址已变更",
        );
    }
    let mut response = tokio::select! {
        _ = state.cancel.cancelled() => error(StatusCode::SERVICE_UNAVAILABLE,"局域网服务已关闭"),
        result = tokio::time::timeout(Duration::from_secs(15 * 60), next.run(request)) => {
            result.unwrap_or_else(|_| error(StatusCode::REQUEST_TIMEOUT,"请求超时，请重试"))
        }
    };
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    headers.insert("referrer-policy", "no-referrer".parse().unwrap());
    headers.insert(
        "content-security-policy",
        "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' blob:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'"
            .parse()
            .unwrap(),
    );
    response
}

fn error(status: StatusCode, message: impl ToString) -> Response {
    (status, Json(json!({"error":message.to_string()}))).into_response()
}

#[derive(Deserialize)]
struct TextRequest {
    text: String,
    request_id: String,
}

fn begin_upload(
    state: &HttpState,
    id: &str,
    name: &str,
    total: Option<u64>,
) -> Result<Option<Value>, String> {
    uuid::Uuid::parse_str(id).map_err(|_| "上传请求标识无效")?;
    let mut inner = state.shared.inner.lock().map_err(|_| "任务状态不可用")?;
    if let Some(existing) = inner_task_mut(&mut inner.tasks, id) {
        if existing.state == "done" {
            return Ok(existing.result.clone());
        }
        if existing.state != "error" {
            return Ok(Some(pending_result(id)));
        }
    }
    inner.tasks.retain(|task| task.id != id);
    while inner.tasks.len() >= 100 {
        let Some(index) = inner.tasks.iter().rposition(|task| {
            !matches!(task.state.as_str(), "receiving" | "waiting" | "delivered")
        }) else {
            return Err("接收任务过多，请稍后重试".into());
        };
        inner.tasks.remove(index);
    }
    inner.tasks.push_front(ipc::LanWebTask {
        id: id.into(),
        name: name.into(),
        state: "receiving".into(),
        received_bytes: 0,
        total_bytes: total,
        recorded: None,
        clipboard_written: None,
        message: "正在接收".into(),
        result: None,
    });
    Ok(None)
}

struct UploadGuard {
    state: HttpState,
    id: String,
}

impl Drop for UploadGuard {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.state.shared.inner.lock()
            && let Some(task) = inner_task_mut(&mut inner.tasks, &self.id)
            && task.state == "receiving"
        {
            task.state = "error".into();
            task.recorded = Some(false);
            task.clipboard_written = Some(false);
            task.message = "接收未完成，请重试".into();
            task.result = Some(task_result(task));
        }
    }
}

fn accept_upload(
    state: &HttpState,
    id: String,
    formats: Vec<ipc::Format>,
    received: u64,
) -> Result<Value, String> {
    if !state.active() {
        return Err("局域网服务已关闭或地址已变更".into());
    }
    let mut inner = state.shared.inner.lock().map_err(|_| "任务状态不可用")?;
    if inner.pending.len() >= MAX_PENDING {
        return Err("等待电脑确认的内容过多，请稍后重试".into());
    }
    let name = inner_task_mut(&mut inner.tasks, &id)
        .map(|task| {
            task.state = "waiting".into();
            task.received_bytes = received;
            task.total_bytes = Some(received);
            task.message = "等待电脑确认".into();
            task.name.clone()
        })
        .ok_or("上传任务已失效")?;
    let upload = ipc::LanWebUpload { id, name, formats };
    let id = upload.id.clone();
    inner.pending.push_back(PendingUpload {
        upload,
        delivered: false,
    });
    Ok(pending_result(&id))
}

async fn upload_text(State(state): State<HttpState>, Json(request): Json<TextRequest>) -> Response {
    if request.text.trim().is_empty() {
        return error(StatusCode::BAD_REQUEST, "请输入要发送的文字");
    }
    if request.text.len() > MAX_TEXT_BODY {
        return error(StatusCode::BAD_REQUEST, "文字超过 1 MiB 限制");
    }
    let _permit = match state.shared.upload_slots.clone().acquire_owned().await {
        Ok(permit) => permit,
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "服务已关闭"),
    };
    let id = request.request_id;
    match begin_upload(&state, &id, "文字", Some(request.text.len() as u64)) {
        Ok(Some(result)) => return Json(result).into_response(),
        Ok(None) => {}
        Err(message) => return error(StatusCode::CONFLICT, message),
    }
    let _upload = UploadGuard {
        state: state.clone(),
        id: id.clone(),
    };
    let format = ipc::Format {
        mime: "text/plain".into(),
        data_base64: base64::engine::general_purpose::STANDARD.encode(request.text.as_bytes()),
    };
    match accept_upload(&state, id, vec![format], request.text.len() as u64) {
        Ok(result) => (StatusCode::ACCEPTED, Json(result)).into_response(),
        Err(message) => error(StatusCode::BAD_REQUEST, message),
    }
}

fn photo_name(raw: &str) -> Result<String, String> {
    let name = raw.rsplit(['/', '\\']).next().unwrap_or("").trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 240
        || name.chars().any(char::is_control)
    {
        return Err("照片文件名无效".into());
    }
    Ok(name.into())
}

async fn upload_photo(
    State(state): State<HttpState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    let total = match headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        Some(total) if total > 0 && total <= MAX_PHOTO_BODY as u64 => total,
        _ => {
            return error(
                StatusCode::LENGTH_REQUIRED,
                "照片大小未知或超过 16 MiB，请重新选择",
            );
        }
    };
    let _permit = match state.shared.upload_slots.clone().acquire_owned().await {
        Ok(permit) => permit,
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "服务已关闭"),
    };
    let mut first = match multipart.next_field().await {
        Ok(Some(field)) if field.name() == Some("request_id") => field,
        _ => return error(StatusCode::BAD_REQUEST, "缺少上传请求标识"),
    };
    let mut id_bytes = Vec::new();
    loop {
        match first.chunk().await {
            Ok(Some(bytes)) if id_bytes.len() + bytes.len() <= 64 => {
                id_bytes.extend_from_slice(&bytes);
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
        Ok(Some(field)) if field.name() == Some("photo") => field,
        _ => return error(StatusCode::BAD_REQUEST, "请选择照片"),
    };
    let name = match photo_name(field.file_name().unwrap_or("")) {
        Ok(name) => name,
        Err(message) => return error(StatusCode::BAD_REQUEST, message),
    };
    match begin_upload(&state, &id, &name, Some(total)) {
        Ok(Some(result)) => return Json(result).into_response(),
        Ok(None) => {}
        Err(message) => return error(StatusCode::CONFLICT, message),
    }
    let _upload = UploadGuard {
        state: state.clone(),
        id: id.clone(),
    };
    let mut data = Vec::new();
    loop {
        let next = tokio::select! {
            _ = state.cancel.cancelled() => return error(StatusCode::SERVICE_UNAVAILABLE,"局域网服务已关闭"),
            result = tokio::time::timeout(Duration::from_secs(60), field.chunk()) => result,
        };
        let bytes = match next {
            Ok(Ok(Some(bytes))) => bytes,
            Ok(Ok(None)) => break,
            Ok(Err(_)) => return error(StatusCode::BAD_REQUEST, "照片上传中断，请重试"),
            Err(_) => return error(StatusCode::REQUEST_TIMEOUT, "接收照片超时"),
        };
        if data.len() + bytes.len() > MAX_CONTENT {
            return error(StatusCode::BAD_REQUEST, "照片超过 16 MiB 限制");
        }
        data.extend_from_slice(&bytes);
        let received = data.len() as u64;
        if let Ok(mut inner) = state.shared.inner.lock()
            && let Some(task) = inner_task_mut(&mut inner.tasks, &id)
        {
            task.received_bytes = received;
        }
    }
    drop(field);
    if data.is_empty() {
        return error(StatusCode::BAD_REQUEST, "照片为空");
    }
    match multipart.next_field().await {
        Ok(None) => {}
        _ => return error(StatusCode::BAD_REQUEST, "每个请求只能上传一张照片"),
    }
    let received = data.len() as u64;
    let formats = match tokio::task::spawn_blocking(move || validate_photo(&data)).await {
        Ok(Ok(formats)) => formats,
        Ok(Err(message)) => return error(StatusCode::BAD_REQUEST, message),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "图片验证失败"),
    };
    match accept_upload(&state, id, formats, received) {
        Ok(result) => (StatusCode::ACCEPTED, Json(result)).into_response(),
        Err(message) => error(StatusCode::BAD_REQUEST, message),
    }
}

fn validate_photo(data: &[u8]) -> Result<Vec<ipc::Format>, String> {
    let reader = image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .map_err(|_| "无法识别照片格式")?;
    let format = reader.format().ok_or("无法识别照片格式")?;
    let (width, height) = reader.into_dimensions().map_err(|_| "无法读取照片尺寸")?;
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 || pixels > MAX_PIXELS {
        return Err("照片像素超过 2400 万限制".into());
    }
    let decoded = image::load_from_memory_with_format(data, format)
        .map_err(|_| "照片数据损坏或当前格式不可用")?;
    let (mime, bytes) = match format {
        image::ImageFormat::Png => ("image/png", data.to_vec()),
        image::ImageFormat::Jpeg => ("image/jpeg", data.to_vec()),
        image::ImageFormat::WebP => ("image/webp", data.to_vec()),
        image::ImageFormat::Tiff => {
            let mut output = std::io::Cursor::new(Vec::new());
            decoded
                .write_to(&mut output, image::ImageFormat::Png)
                .map_err(|_| "照片转换失败")?;
            ("image/png", output.into_inner())
        }
        _ => return Err("请选择 JPEG、PNG、TIFF 或 WebP 图片".into()),
    };
    if bytes.len() > MAX_CONTENT {
        return Err("转换后的照片超过 16 MiB 限制".into());
    }
    Ok(vec![ipc::Format {
        mime: mime.into(),
        data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn service(root: &Path) -> LanWebService {
        LanWebService::with_options(
            root.join("lan-web.json"),
            "Test PC".into(),
            "127.0.0.1".into(),
            0,
            Arc::new(|| vec![Ipv4Addr::LOCALHOST]),
        )
    }

    async fn wait_running(service: &LanWebService) -> String {
        for _ in 0..100 {
            let state = service.state();
            if state.running {
                return state.address.unwrap();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("LAN web did not start: {:?}", service.state().error);
    }

    async fn request(
        address: &str,
        method: &str,
        path: &str,
        content_type: Option<&str>,
        body: &[u8],
        host: Option<&str>,
    ) -> (u16, String) {
        let authority = address.strip_prefix("http://").unwrap();
        let mut stream = tokio::net::TcpStream::connect(authority).await.unwrap();
        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\n",
            host.unwrap_or(authority),
            body.len()
        );
        if let Some(content_type) = content_type {
            request.push_str(&format!("Content-Type: {content_type}\r\n"));
        }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();
        stream.write_all(body).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        let response = String::from_utf8_lossy(&response).into_owned();
        let status = response.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, response)
    }

    #[tokio::test]
    async fn text_upload_waits_for_desktop_commit_and_persists_settings() {
        let root = std::env::temp_dir().join(format!("deskfuse-lan-{}", uuid::Uuid::new_v4()));
        let service = service(&root);
        service.resume_session();
        service.configure(true).unwrap();
        let address = wait_running(&service).await;
        let id = uuid::Uuid::new_v4().to_string();
        let body = serde_json::to_vec(&json!({"text":"hello LAN","request_id":id})).unwrap();
        let (status, _) = request(
            &address,
            "POST",
            "/api/text",
            Some("application/json"),
            &body,
            None,
        )
        .await;
        assert_eq!(status, 202);
        let deliveries = service.deliveries();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].id, id);
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(&deliveries[0].formats[0].data_base64)
                .unwrap(),
            b"hello LAN"
        );
        assert!(service.deliveries().is_empty());
        assert!(service.commit(&id, true, true));
        assert!(!service.commit(&id, true, true));
        assert_eq!(service.state().tasks[0].state, "done");
        let (status, response) = request(&address, "GET", "/api/tasks", None, &[], None).await;
        assert_eq!(status, 200);
        assert!(response.contains("done"));
        service.configure(false).unwrap();
        service.configure(true).unwrap();
        let _ = wait_running(&service).await;
        service.pause_session();
        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn photo_upload_multipart_reaches_desktop() {
        let root = std::env::temp_dir().join(format!("deskfuse-lan-{}", uuid::Uuid::new_v4()));
        let service = service(&root);
        service.resume_session();
        service.configure(true).unwrap();
        let address = wait_running(&service).await;
        let id = uuid::Uuid::new_v4().to_string();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(1, 1)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let boundary = "deskfuse-test-boundary";
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"request_id\"\r\n\r\n{id}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"photo\"; filename=\"pixel.png\"\r\nContent-Type: image/png\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(&png.into_inner());
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let (status, response) = request(
            &address,
            "POST",
            "/api/photos",
            Some(&format!("multipart/form-data; boundary={boundary}")),
            &body,
            None,
        )
        .await;
        assert_eq!(status, 202, "{response}");
        let deliveries = service.deliveries();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].formats[0].mime, "image/png");
        assert!(service.commit(&id, true, true));
        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn http_guard_rejects_foreign_host_and_pause_cancels_pending() {
        let root = std::env::temp_dir().join(format!("deskfuse-lan-{}", uuid::Uuid::new_v4()));
        let service = service(&root);
        service.resume_session();
        service.configure(true).unwrap();
        let address = wait_running(&service).await;
        let (status, _) = request(&address, "GET", "/", None, &[], Some("example.com")).await;
        assert_eq!(status, 403);
        let id = uuid::Uuid::new_v4().to_string();
        let body = serde_json::to_vec(&json!({"text":"discard on lock","request_id":id})).unwrap();
        let (status, _) = request(
            &address,
            "POST",
            "/api/text",
            Some("application/json"),
            &body,
            None,
        )
        .await;
        assert_eq!(status, 202);
        assert_eq!(service.deliveries().len(), 1);
        service.pause_session();
        assert!(!service.commit(&id, true, true));
        assert!(service.deliveries().is_empty());
        assert_eq!(service.state().tasks[0].state, "error");
        drop(service);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn photo_validation_checks_real_format_and_converts_tiff() {
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(1, 1)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let formats = validate_photo(&png.into_inner()).unwrap();
        assert_eq!(formats[0].mime, "image/png");
        let mut tiff = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(1, 1)
            .write_to(&mut tiff, image::ImageFormat::Tiff)
            .unwrap();
        assert_eq!(
            validate_photo(&tiff.into_inner()).unwrap()[0].mime,
            "image/png"
        );
        assert!(validate_photo(b"not a photo").is_err());
    }
}
