//! Owner-controlled HTTP projection of the existing library.
//! No device trust is created here: web clients can only read content and submit uploads.
mod content;
mod upload;
use crate::tokio_bridge::TokioBridge;
use axum::{
    Router,
    body::Body,
    extract::{DefaultBodyLimit, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};
use tokio::sync::{Notify, Semaphore};
use tokio_util::sync::CancellationToken;

pub(super) type Result<T> = std::result::Result<T, String>;
const MAX_PHOTO_BODY: usize = 256 * 1024 * 1024 + 64 * 1024;
const MAX_TEXT_BODY: usize = 1024 * 1024;
#[derive(Clone, Default, Deserialize, Serialize)]
struct Settings {
    enabled: bool,
    preferred_interface: Option<String>,
    // The legacy `port` was the last randomly assigned port, not user intent.
    #[serde(default, rename = "configured_port")]
    port: u16,
}
#[derive(Clone, Serialize)]
struct Interface {
    name: String,
    address: String,
    label: String,
}
#[derive(Clone, Serialize)]
pub struct UploadTask {
    pub id: String,
    pub name: String,
    pub state: String,
    pub received_bytes: u64,
    pub total_bytes: Option<u64>,
    pub clipboard_written: Option<bool>,
    pub message: String,
    #[serde(skip)]
    result: Option<Value>,
}
struct Inner {
    settings: Settings,
    settings_epoch: u64,
    running: bool,
    address: Option<String>,
    addresses: Vec<String>,
    local_addresses: Vec<String>,
    listen_address: Option<String>,
    interfaces: Vec<Interface>,
    error: Option<String>,
    tasks: VecDeque<UploadTask>,
}
type WatchedFile = (String, Option<(u64, std::time::SystemTime)>);
type LiveServer = (
    String,
    u16,
    CancellationToken,
    Arc<AtomicBool>,
    tokio::task::JoinHandle<()>,
);

struct Shared {
    inner: Mutex<Inner>,
    bridge: Weak<TokioBridge>,
    path: PathBuf,
    changed: Notify,
    stop: CancellationToken,
    uploads: Arc<Semaphore>,
    reads: Arc<Semaphore>,
    watched_files: Mutex<VecDeque<WatchedFile>>,
    file_revision: AtomicU64,
}
#[derive(Clone)]
struct HttpState {
    shared: Arc<Shared>,
    bind_ip: String,
    port: u16,
    settings_epoch: u64,
    cancel: CancellationToken,
    cancelled: Arc<AtomicBool>,
}

/// Captured on the serialized pasteboard worker before accepting an upload.
pub struct LanContext {
    pub change_count: i64,
    pub local_id: String,
    pub local_name: String,
}
pub struct LanCommitResult {
    pub item_id: u64,
    pub clipboard_written: bool,
    pub message: String,
}

pub struct LanWebService {
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}
impl LanWebService {
    pub fn new(bridge: &Arc<TokioBridge>) -> Result<Self> {
        let path = unidesk_store::SQLiteStore::default_path()
            .map_err(|e| e.to_string())?
            .with_file_name("lan-web.json");
        let (settings, error) = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(s) => (s, None),
                Err(_) => (
                    Settings::default(),
                    Some("局域网设置读取失败，请重新开启服务".into()),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Settings::default(), None),
            Err(_) => (Settings::default(), Some("局域网设置无法读取".into())),
        };
        let shared = Arc::new(Shared {
            inner: Mutex::new(Inner {
                settings,
                settings_epoch: 0,
                running: false,
                address: None,
                addresses: vec![],
                local_addresses: vec![],
                listen_address: None,
                interfaces: vec![],
                error,
                tasks: VecDeque::new(),
            }),
            bridge: Arc::downgrade(bridge),
            path,
            changed: Notify::new(),
            stop: CancellationToken::new(),
            uploads: Arc::new(Semaphore::new(1)),
            reads: Arc::new(Semaphore::new(4)),
            watched_files: Mutex::new(VecDeque::new()),
            file_revision: AtomicU64::new(0),
        });
        let task = shared.clone();
        let worker = std::thread::Builder::new()
            .name("unidesk-lan-web".into())
            .spawn(move || {
                match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(rt) => {
                        rt.block_on(serve(task));
                        rt.shutdown_timeout(Duration::from_secs(5));
                    }
                    Err(e) => {
                        if let Ok(mut inner) = task.inner.lock() {
                            inner.error = Some(format!("网页服务启动失败：{e}"));
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            shared,
            worker: Mutex::new(Some(worker)),
        })
    }
    pub fn state(&self) -> Result<String> {
        self.shared
            .projection()
            .and_then(|v| serde_json::to_string(&v).map_err(|e| e.to_string()))
    }
    pub fn configure(&self, enabled: bool, interface: Option<String>, port: u16) -> Result<String> {
        if self.shared.stop.is_cancelled() {
            return Err("引擎已停止".into());
        }
        let mut inner = self.shared.inner.lock().map_err(|_| "网页服务状态不可用")?;
        if enabled
            && interface != inner.settings.preferred_interface
            && let Some(name) = &interface
            && !inner.interfaces.iter().any(|i| &i.name == name)
        {
            return Err("所选网络接口已不可用".into());
        }
        let next = Settings {
            enabled,
            preferred_interface: interface,
            port,
        };
        save_settings(&self.shared.path, &next)?;
        // Hide the old QR immediately. Reconciliation below publishes only a bound listener.
        if inner.settings.enabled != enabled
            || inner.settings.preferred_interface != next.preferred_interface
            || inner.settings.port != next.port
        {
            inner.settings_epoch = inner.settings_epoch.wrapping_add(1);
            inner.running = false;
            inner.address = None;
            inner.addresses.clear();
            inner.listen_address = None;
        }
        inner.settings = next;
        inner.error = None;
        drop(inner);
        self.shared.changed.notify_one();
        self.state()
    }
    pub fn shutdown(&self) {
        self.shared.stop.cancel();
        if let Ok(mut w) = self.worker.lock()
            && let Some(w) = w.take()
        {
            let _ = w.join();
        }
    }
}
impl Drop for LanWebService {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn save_settings(path: &std::path::Path, settings: &Settings) -> Result<()> {
    let bytes = serde_json::to_vec(settings).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)
        .and_then(|_| std::fs::rename(&tmp, path))
        .map_err(|e| format!("无法保存局域网设置：{e}"))
}
impl Shared {
    fn file_stamp(path: &str) -> Option<(u64, std::time::SystemTime)> {
        let meta = std::fs::symlink_metadata(path).ok()?;
        if meta.file_type().is_symlink() || (!meta.is_file() && !meta.is_dir()) {
            return None;
        }
        Some((meta.len(), meta.modified().ok()?))
    }
    fn observe_files(&self, paths: &[String]) -> bool {
        let mut watched = self
            .watched_files
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut available = true;
        for path in paths {
            let stamp = Self::file_stamp(path);
            available &= stamp.is_some();
            if let Some(index) = watched.iter().position(|(p, _)| p == path) {
                let old = watched.remove(index).unwrap();
                if old.1 != stamp {
                    self.file_revision.fetch_add(1, Ordering::Relaxed);
                }
            }
            watched.push_back((path.clone(), stamp));
        }
        while watched.len() > 400 {
            watched.pop_front();
        }
        available
    }
    fn refresh_files(&self) -> u64 {
        let mut watched = self
            .watched_files
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (path, stamp) in watched.iter_mut() {
            let current = Self::file_stamp(path);
            if *stamp != current {
                *stamp = current;
                self.file_revision.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.file_revision.load(Ordering::Relaxed)
    }
    fn bridge(&self) -> Result<Arc<TokioBridge>> {
        self.bridge.upgrade().ok_or_else(|| "引擎已停止".into())
    }
    fn projection(&self) -> Result<Value> {
        let i = self.inner.lock().map_err(|_| "网页服务状态不可用")?;
        Ok(
            json!({"enabled":i.settings.enabled,"running":i.running,"address":i.address,"addresses":i.addresses,"listen_address":i.listen_address,"port":i.settings.port,"preferred_interface":i.settings.preferred_interface,"interfaces":i.interfaces,"error":i.error,"tasks":i.tasks}),
        )
    }
    fn update_task(&self, id: &str, update: impl FnOnce(&mut UploadTask)) {
        if let Ok(mut i) = self.inner.lock()
            && let Some(t) = i.tasks.iter_mut().find(|t| t.id == id)
        {
            update(t);
        }
    }
}
async fn serve(shared: Arc<Shared>) {
    let mut live: Option<LiveServer> = None;
    loop {
        let mut seen_interfaces = std::collections::BTreeSet::new();
        let interfaces = crate::platform::lan_web::interfaces()
            .unwrap_or_default()
            .into_iter()
            .filter(|i| seen_interfaces.insert(i.name.clone()))
            .map(|i| Interface {
                name: i.name,
                address: i.address,
                label: i.label,
            })
            .collect::<Vec<_>>();
        let (desired, settings_epoch) = {
            let mut i = shared
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            i.interfaces = interfaces;
            i.local_addresses = crate::platform::lan_web::all_ipv4_interfaces()
                .unwrap_or_default()
                .into_iter()
                .map(|v| v.address)
                .collect();
            let desired = if i.settings.enabled {
                match &i.settings.preferred_interface {
                    None => Some(("0.0.0.0".to_owned(), i.settings.port)),
                    Some(name) => i
                        .interfaces
                        .iter()
                        .find(|v| &v.name == name)
                        .map(|v| (v.address.clone(), i.settings.port)),
                }
            } else {
                None
            };
            (desired, i.settings_epoch)
        };
        let unpublished = shared.inner.lock().map(|i| !i.running).unwrap_or(true);
        let restart = unpublished && live.is_some()
            || match (&live, &desired) {
                (Some((ip, _, _, _, h)), Some((next, _))) => ip != next || h.is_finished(),
                (None, None) => false,
                _ => true,
            };
        if restart || shared.stop.is_cancelled() {
            if let Some((_, _, cancel, flag, mut handle)) = live.take() {
                flag.store(true, Ordering::Release);
                cancel.cancel();
                if tokio::time::timeout(Duration::from_secs(3), &mut handle)
                    .await
                    .is_err()
                {
                    handle.abort();
                    let _ = handle.await;
                }
            }
            let mut i = shared
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            i.running = false;
            i.address = None;
            i.addresses.clear();
            i.listen_address = None;
        }
        if shared.stop.is_cancelled() {
            break;
        }
        if live.is_none() {
            if let Some((ip, port)) = desired {
                let listener = tokio::net::TcpListener::bind((ip.as_str(), port)).await;
                match listener {
                    Ok(listener) => {
                        let Ok(local) = listener.local_addr() else {
                            continue;
                        };
                        let cancel = CancellationToken::new();
                        let cancelled = Arc::new(AtomicBool::new(false));
                        let state = HttpState {
                            shared: shared.clone(),
                            bind_ip: ip.clone(),
                            port: local.port(),
                            settings_epoch,
                            cancel: cancel.clone(),
                            cancelled: cancelled.clone(),
                        };
                        let app = router(state);
                        let stop = cancel.clone();
                        let handle = tokio::spawn(async move {
                            let _ = axum::serve(listener, app)
                                .with_graceful_shutdown(stop.cancelled_owned())
                                .await;
                        });
                        let mut i = shared
                            .inner
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        // Settings may change while the old listener drains or bind awaits.
                        // Never publish a QR for an obsolete enable/interface request.
                        if i.settings_epoch != settings_epoch
                            || !i.settings.enabled
                            || shared.stop.is_cancelled()
                        {
                            cancelled.store(true, Ordering::Release);
                            cancel.cancel();
                            handle.abort();
                            continue;
                        }
                        i.error = None;
                        i.running = true;
                        i.listen_address = Some(local.to_string());
                        live = Some((ip, local.port(), cancel, cancelled, handle));
                    }
                    Err(e) => {
                        shared
                            .inner
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .error = Some(format!("无法监听局域网地址：{e}"));
                    }
                }
            } else {
                let mut i = shared
                    .inner
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if i.settings.enabled {
                    i.error =
                        Some("所选网络接口没有可用的 IPv4 地址，请选择其他接口或所有接口".into());
                }
            }
        }
        if let Some((ip, port, _, _, _)) = &live {
            let mut i = shared
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if i.running {
                // Prefer physical LAN addresses for QR display, then include all
                // other assigned addresses accepted by the wildcard listener.
                let candidates = i
                    .interfaces
                    .iter()
                    .map(|v| v.address.clone())
                    .chain(i.local_addresses.iter().cloned());
                let mut addresses = Vec::new();
                for address in candidates {
                    // Loopback is accepted locally but cannot be scanned by a phone.
                    if address
                        .parse::<std::net::Ipv4Addr>()
                        .is_ok_and(|ip| ip.is_loopback())
                    {
                        continue;
                    }
                    if ip != "0.0.0.0" && &address != ip {
                        continue;
                    }
                    let url = if *port == 80 {
                        format!("http://{address}")
                    } else {
                        format!("http://{address}:{port}")
                    };
                    if !addresses.contains(&url) {
                        addresses.push(url);
                    }
                }
                i.address = addresses.first().cloned();
                i.addresses = addresses;
            }
        }
        tokio::select! {_ = shared.stop.cancelled()=>{},_ = shared.changed.notified()=>{},_ = tokio::time::sleep(Duration::from_secs(1))=>{}}
    }
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
        .route("/api/state", get(content::state))
        .route("/api/items", get(content::items))
        .route("/api/items/{id}", get(content::item))
        .route("/api/items/{id}/image", get(content::image))
        .route("/api/items/{id}/download", get(content::download))
        .route("/api/tasks", get(tasks))
        .route(
            "/api/text",
            post(upload::text).layer(DefaultBodyLimit::max(MAX_TEXT_BODY)),
        )
        .route(
            "/api/photos",
            post(upload::photo).layer(DefaultBodyLimit::max(MAX_PHOTO_BODY)),
        )
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}
fn asset(mime: &'static str, content: &'static str) -> Response {
    ([(header::CONTENT_TYPE, mime)], content).into_response()
}
async fn tasks(State(s): State<HttpState>) -> Response {
    match s.shared.projection() {
        Ok(v) => axum::Json(json!({"tasks":v["tasks"]})).into_response(),
        Err(e) => error(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}
async fn guard(State(s): State<HttpState>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok());
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|h| h.to_str().ok());
    let authority = host.and_then(|h| h.parse::<axum::http::uri::Authority>().ok());
    let allowed_host = authority.as_ref().is_some_and(|a| {
        a.port_u16().unwrap_or(80) == s.port
            && a.host().parse::<std::net::Ipv4Addr>().is_ok()
            && if s.bind_ip == "0.0.0.0" {
                s.shared
                    .inner
                    .lock()
                    .map(|i| i.local_addresses.iter().any(|ip| ip == a.host()))
                    .unwrap_or(false)
            } else {
                a.host() == s.bind_ip
            }
    });
    let expected = format!("http://{}", host.unwrap_or_default());
    if !allowed_host
        || origin.is_some_and(|o| o != expected)
        || request
            .headers()
            .get("sec-fetch-site")
            .is_some_and(|v| v == "cross-site")
    {
        return error(StatusCode::FORBIDDEN, "请从 Mac 上显示的局域网地址进入");
    }
    let active = s
        .shared
        .inner
        .lock()
        .map(|i| i.settings.enabled && i.running && i.settings_epoch == s.settings_epoch)
        .unwrap_or(false);
    if !active || s.cancelled.load(Ordering::Acquire) {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "局域网服务已关闭或地址已变更",
        );
    }
    let mut response = tokio::select! {
        _=s.cancel.cancelled()=>error(StatusCode::SERVICE_UNAVAILABLE,"局域网服务已关闭"),
        r=tokio::time::timeout(Duration::from_secs(15*60),next.run(request))=>r.unwrap_or_else(|_|error(StatusCode::REQUEST_TIMEOUT,"请求超时，请重试"))
    };
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    headers.insert("referrer-policy", "no-referrer".parse().unwrap());
    headers.insert("content-security-policy","default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' blob:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'".parse().unwrap());
    response
}
fn error(status: StatusCode, message: impl ToString) -> Response {
    (status, axum::Json(json!({"error":message.to_string()}))).into_response()
}
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| "后台任务未能完成".to_owned())?
}
