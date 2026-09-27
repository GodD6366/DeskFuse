use super::*;
use axum::extract::{Path, Query};
use axum::http::HeaderValue;
use std::{
    collections::{BTreeSet, HashMap},
    io::{Read, Write},
    path::{Path as FilePath, PathBuf},
};
use tokio_util::io::ReaderStream;
use unidesk_platform::PasteboardSnapshot;
use unidesk_transfer::TransferId;

pub(super) async fn state(State(s): State<HttpState>) -> Response {
    let bridge = match s.shared.bridge() {
        Ok(b) => b,
        Err(e) => return error(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    let Ok(permit) = s.shared.reads.clone().acquire_owned().await else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "服务已关闭");
    };
    match blocking(move|| {
        let _permit=permit;
        let file_revision=s.shared.refresh_files();
        let library=crate::clipboard_library::global()?;
        let mut v:Value=serde_json::from_str(&library.query(r#"{"op":"state"}"#)?).map_err(|e|e.to_string())?;
        let context=bridge.lan_web_context()?;
        let devices=v["devices"].as_array_mut().ok_or("设备列表不可用")?;
        if let Some(d)=devices.iter_mut().find(|d|d["id"]==context.local_id){d["name"]=json!(context.local_name);}else{devices.push(json!({"id":context.local_id,"name":context.local_name}));}
        Ok(json!({"revision":format!("{}:{file_revision}",v["revision"]),"lists":v["lists"],"devices":v["devices"],"name":context.local_name}))
    }).await {Ok(v)=>axum::Json(v).into_response(),Err(e)=>error(StatusCode::SERVICE_UNAVAILABLE,e)}
}
pub(super) async fn items(
    State(s): State<HttpState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Ok(_permit) = s.shared.reads.clone().acquire_owned().await else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "服务已关闭");
    };
    match blocking(move|| {
        let _permit=_permit;
        let mut request=json!({"offset":q.get("offset").and_then(|n|n.parse::<u64>().ok()).unwrap_or(0).min(100_000),"limit":q.get("limit").and_then(|n|n.parse::<u64>().ok()).unwrap_or(40).clamp(1,100)});
        for key in ["query","list_id","source"] {if let Some(v)=q.get(key).filter(|v|!v.is_empty()){if v.len()>4096{return Err("筛选条件过长".into());}request[key]=json!(v);}}
        let mut v:Value=serde_json::from_str(&crate::clipboard_library::global()?.query(&request.to_string())?).map_err(|e|e.to_string())?;
        let bridge=s.shared.bridge()?;
        let local_id=bridge.lan_web_context()?.local_id;
        if let Some(items)=v["items"].as_array_mut(){for item in items {
            if let Some(id)=item["id"].as_u64(){
                item["id"]=json!(id.to_string());
                if item["file_count"].as_u64().unwrap_or(0)>0 {
                    let paths=crate::clipboard_library::global()?.snapshot(id)
                        .and_then(|snap|crate::library_files::local_paths_for_preview(&snap,&local_id)).ok().flatten();
                    item["available"]=json!(paths.as_ref().is_some_and(|paths|!paths.is_empty() && s.shared.observe_files(paths)));
                }
            }
        }}
        Ok(v)
    }).await {Ok(v)=>axum::Json(v).into_response(),Err(e)=>error(StatusCode::BAD_REQUEST,e)}
}
fn snapshot(id: u64, bridge: &TokioBridge) -> Result<PasteboardSnapshot> {
    let source = crate::clipboard_library::global()?.snapshot(id)?;
    // Resolve local archived references only; browsing must not implicitly fetch remote files.
    if crate::library_files::reference(&source)?.is_some() {
        crate::library_files::resolve_local(&source, &bridge.lan_web_context()?.local_id)?
            .ok_or_else(|| "内容尚未保存在这台 Mac，请先在 Mac 获取".into())
    } else {
        Ok(source)
    }
}
pub(super) fn image_kind(path: &str) -> Option<&'static str> {
    match FilePath::new(path)
        .extension()?
        .to_str()?
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Some("public.png"),
        "jpg" | "jpeg" => Some("public.jpeg"),
        "heic" | "heif" => Some("public.heic"),
        "tif" | "tiff" => Some("public.tiff"),
        "gif" => Some("com.compuserve.gif"),
        "webp" => Some("org.webmproject.webp"),
        _ => None,
    }
}
fn has_image(s: &PasteboardSnapshot) -> bool {
    s.png.is_some()
        || s.items.iter().flat_map(|i| &i.representations).any(|r| {
            matches!(
                r.type_name.as_str(),
                "public.png" | "public.jpeg" | "public.tiff" | "public.heic"
            )
        })
        || s.file_urls.first().is_some_and(|p| image_kind(p).is_some())
}
fn files_available(s: &PasteboardSnapshot) -> bool {
    s.file_urls.iter().all(|p| {
        std::fs::symlink_metadata(p)
            .is_ok_and(|m| !m.file_type().is_symlink() && (m.is_file() || m.is_dir()))
    })
}
pub(super) async fn item(State(s): State<HttpState>, Path(id): Path<u64>) -> Response {
    let Ok(_permit) = s.shared.reads.clone().acquire_owned().await else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "服务已关闭");
    };
    let b = match s.shared.bridge() {
        Ok(b) => b,
        Err(e) => return error(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    match blocking(move|| {
        let _permit=_permit;
        let snap=snapshot(id,&b)?;let available=files_available(&snap);let image=has_image(&snap);
        let files=snap.file_urls.iter().map(|p|json!({"name":FilePath::new(p).file_name().unwrap_or_default().to_string_lossy()})).collect::<Vec<_>>();
        Ok(json!({"id":id.to_string(),"text":if snap.file_urls.is_empty(){snap.text}else{None},"has_image":image,"file_count":files.len(),"files":files,"available":available,"image_url":(image&&available).then(||format!("/api/items/{id}/image")),"download_url":(available&&(image||!snap.file_urls.is_empty())).then(||format!("/api/items/{id}/download"))}))
    }).await{Ok(v)=>axum::Json(v).into_response(),Err(e)=>error(StatusCode::NOT_FOUND,e)}
}
fn render_image(snap: &PasteboardSnapshot, size: u32) -> Result<Vec<u8>> {
    if let Some(path) = snap.file_urls.first() {
        return crate::platform::lan_web::image_png(FilePath::new(path), size);
    }
    if let Some(bytes) = &snap.png {
        return crate::platform::lan_web::data_image_png(bytes, size);
    }
    for rep in snap.items.iter().flat_map(|i| &i.representations) {
        if matches!(
            rep.type_name.as_str(),
            "public.png" | "public.jpeg" | "public.tiff" | "public.heic"
        ) {
            return crate::platform::lan_web::data_image_png(&rep.data, size);
        }
    }
    Err("此记录没有可预览的图片".into())
}
pub(super) async fn image(
    State(s): State<HttpState>,
    Path(id): Path<u64>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Ok(_permit) = s.shared.reads.clone().acquire_owned().await else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "服务已关闭");
    };
    let b = match s.shared.bridge() {
        Ok(b) => b,
        Err(e) => return error(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    let size = q
        .get("size")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1600)
        .clamp(160, 2048);
    match blocking(move || {
        let _permit = _permit;
        let snap = snapshot(id, &b)?;
        let _lease = Export::new(b, snap.file_urls.clone())?;
        render_image(&snap, size)
    })
    .await
    {
        Ok(bytes) => ([(header::CONTENT_TYPE, "image/png")], bytes).into_response(),
        Err(e) => error(StatusCode::NOT_FOUND, e),
    }
}
/// Export ownership follows the response body, not just the handler future.
struct Export {
    bridge: Arc<TokioBridge>,
    lease: TransferId,
    temporary: Option<(TransferId, PathBuf)>,
}
impl Export {
    fn new(bridge: Arc<TokioBridge>, paths: Vec<String>) -> Result<Self> {
        let lease = bridge.retain_transfer_cache_paths(paths)?;
        Ok(Self {
            bridge,
            lease,
            temporary: None,
        })
    }
}
impl Drop for Export {
    fn drop(&mut self) {
        if let Some((id, root)) = &self.temporary {
            let _ = std::fs::remove_dir_all(root);
            let _ = self.bridge.workflow_cache_finish(*id);
        }
        self.bridge.release_transfer_cache_paths(self.lease);
    }
}
struct Prepared {
    file: std::fs::File,
    name: String,
    len: u64,
    export: Export,
}
#[derive(Clone)]
struct ZipEntry {
    source: PathBuf,
    name: String,
    size: u64,
    modified: Option<std::time::SystemTime>,
    directory: bool,
}
fn gather(
    path: &FilePath,
    name: String,
    entries: &mut Vec<ZipEntry>,
    cancel: &AtomicBool,
) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        return Err("下载已取消".into());
    }
    if entries.len() >= 1_000_000 {
        return Err("文件数量过多，请分批下载".into());
    }
    let meta = std::fs::symlink_metadata(path).map_err(|_| "文件已不可用")?;
    if meta.file_type().is_symlink() || (!meta.is_file() && !meta.is_dir()) {
        return Err("下载不支持符号链接或特殊文件".into());
    }
    entries.push(ZipEntry {
        source: path.to_owned(),
        name: name.clone(),
        size: meta.len(),
        modified: meta.modified().ok(),
        directory: meta.is_dir(),
    });
    if meta.is_dir() {
        for child in std::fs::read_dir(path).map_err(|e| e.to_string())? {
            let child = child.map_err(|e| e.to_string())?;
            gather(
                &child.path(),
                format!("{name}/{}", child.file_name().to_string_lossy()),
                entries,
                cancel,
            )?;
        }
    }
    Ok(())
}
fn prepare_download(id: u64, b: Arc<TokioBridge>, cancel: Arc<AtomicBool>) -> Result<Prepared> {
    let snap = snapshot(id, &b)?;
    let mut export = Export::new(b.clone(), snap.file_urls.clone())?;
    if snap.file_urls.len() == 1 && FilePath::new(&snap.file_urls[0]).is_file() {
        let path = FilePath::new(&snap.file_urls[0]);
        if std::fs::symlink_metadata(path)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("下载不支持符号链接".into());
        }
        let file = std::fs::File::open(path).map_err(|_| "文件已不可用")?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        return Ok(Prepared {
            file,
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            len,
            export,
        });
    }
    if snap.file_urls.is_empty() {
        let (bytes, ext) = if let Some(png) = snap.png {
            (png, "png")
        } else if let Some(r) = snap
            .items
            .iter()
            .flat_map(|i| &i.representations)
            .find(|r| {
                matches!(
                    r.type_name.as_str(),
                    "public.png" | "public.jpeg" | "public.tiff" | "public.heic"
                )
            })
        {
            (
                r.data.clone(),
                match r.type_name.as_str() {
                    "public.jpeg" => "jpg",
                    "public.tiff" => "tiff",
                    "public.heic" => "heic",
                    _ => "png",
                },
            )
        } else {
            return Err("此记录没有可下载的文件".into());
        };
        let temp = TransferId::new();
        let root = b.workflow_cache_reserve(temp, bytes.len() as u64)?;
        export.temporary = Some((temp, root.clone()));
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let name = format!("uniDesk-{id}.{ext}");
        let path = root.join(&name);
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        return Ok(Prepared {
            file,
            name,
            len,
            export,
        });
    }
    let mut entries = vec![];
    let mut names = BTreeSet::new();
    for source in &snap.file_urls {
        let path = FilePath::new(source);
        let raw = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut name = raw.clone();
        let mut n = 2;
        while !names.insert(name.clone()) {
            name = format!("{n}-{raw}");
            n += 1;
        }
        gather(path, name, &mut entries, &cancel)?;
    }
    let required = entries
        .iter()
        .filter(|e| !e.directory)
        .try_fold(64 * 1024u64, |a, e| {
            a.checked_add(e.size).ok_or("文件大小超出范围")
        })?
        .saturating_add(
            entries
                .iter()
                .map(|e| (e.name.len() as u64) * 2 + 512)
                .sum::<u64>(),
        );
    let temp = TransferId::new();
    let root = b.workflow_cache_reserve(temp, required)?;
    export.temporary = Some((temp, root.clone()));
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let path = root.join("download.zip");
    let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true);
    let mut buffer = vec![0u8; 256 * 1024];
    for e in entries {
        if cancel.load(Ordering::Acquire) {
            return Err("下载已取消".into());
        }
        if e.directory {
            zip.add_directory(format!("{}/", e.name), options)
                .map_err(|e| e.to_string())?;
            continue;
        }
        let meta = std::fs::symlink_metadata(&e.source).map_err(|_| "文件已不可用")?;
        if !meta.is_file() || meta.len() != e.size || meta.modified().ok() != e.modified {
            return Err("源文件已变化，请重新下载".into());
        }
        let mut input = std::fs::File::open(&e.source).map_err(|_| "文件已不可用")?;
        zip.start_file(e.name, options).map_err(|e| e.to_string())?;
        let mut copied = 0u64;
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("下载已取消".into());
            }
            let n = input.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            copied += n as u64;
            if copied > e.size {
                return Err("源文件已变化，请重新下载".into());
            }
            zip.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        }
        if copied != e.size
            || input.metadata().map_err(|e| e.to_string())?.modified().ok() != e.modified
        {
            return Err("源文件已变化，请重新下载".into());
        }
    }
    zip.finish().map_err(|e| e.to_string())?;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    Ok(Prepared {
        file,
        name: format!("uniDesk-{id}.zip"),
        len,
        export,
    })
}
struct DownloadCancel(Arc<AtomicBool>);
impl Drop for DownloadCancel {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}
pub(super) async fn download(State(s): State<HttpState>, Path(id): Path<u64>) -> Response {
    let Ok(permit) = s.shared.reads.clone().acquire_owned().await else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "服务已关闭");
    };
    let b = match s.shared.bridge() {
        Ok(b) => b,
        Err(e) => return error(StatusCode::SERVICE_UNAVAILABLE, e),
    };
    let flag = Arc::new(AtomicBool::new(false));
    let request_cancel = DownloadCancel(flag.clone());
    let (prepared, permit) =
        match blocking(move || prepare_download(id, b, flag).map(|p| (p, permit))).await {
            Ok(p) => p,
            Err(e) => return error(StatusCode::NOT_FOUND, e),
        };
    let disposition = format!(
        "attachment; filename=\"uniDesk-download\"; filename*=UTF-8''{}",
        url::form_urlencoded::byte_serialize(prepared.name.as_bytes())
            .collect::<String>()
            .replace('+', "%20")
    );
    let stream = ReaderStream::new(tokio::fs::File::from_std(prepared.file));
    let cancel = s.cancel.clone();
    let body = futures::stream::unfold(
        (stream, prepared.export, permit, cancel, request_cancel),
        |(mut stream, export, permit, cancel, request_cancel)| async move {
            use futures::StreamExt;
            let item = tokio::select! {_=cancel.cancelled()=>None,item=stream.next()=>item};
            item.map(|item| (item, (stream, export, permit, cancel, request_cancel)))
        },
    );
    let mut response = Response::new(Body::from_stream(body));
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(prepared.len));
    if let Ok(v) = HeaderValue::from_str(&disposition) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    response
}
