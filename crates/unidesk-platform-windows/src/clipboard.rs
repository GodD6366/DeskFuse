//! Windows clipboard adapter. A dedicated thread owns the message-only HWND.
//!
//! Only documented HGLOBAL-backed formats are locked: never enumerate arbitrary
//! clipboard handles (bitmaps, palettes and OLE interfaces are not global memory).
#![allow(unsafe_code)]

use crate::clipboard_formats::{self as formats, MAX_FORMAT_BYTES};
use std::{cell::Cell, ptr, sync::mpsc, thread, time::Duration};
use unidesk_platform::{
    Pasteboard, PasteboardItem, PasteboardRepresentation, PasteboardSnapshot, PlatformError,
    PlatformResult, REMOTE_ORIGIN_MARKER_TYPE,
};
use windows_sys::Win32::{
    Foundation::{GlobalFree, HGLOBAL, HWND},
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardOwner,
            GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
            RegisterClipboardFormatW, SetClipboardData,
        },
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
    },
    UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, HWND_MESSAGE, MSG, PM_REMOVE,
        PeekMessageW, TranslateMessage,
    },
};

const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
const CF_DIB: u32 = 8;
const CF_DIBV5: u32 = 17;
type Job = Box<dyn FnOnce(&Worker) + Send>;

/// Native clipboard access is marshalled to one window-owning thread. No native
/// handle crosses threads and no clipboard operations execute on GPUI's thread.
pub struct WindowsPasteboard {
    sender: Option<mpsc::Sender<Job>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl WindowsPasteboard {
    /// Start the native provider. Failure is reported, never replaced with a
    /// memory clipboard on a real desktop.
    ///
    /// # Errors
    /// Returns a clipboard error if the worker, native window or formats cannot be initialized.
    pub fn new() -> PlatformResult<Self> {
        let (sender, receiver) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("unidesk-clipboard".into())
            .spawn(move || {
                let native = match Worker::new() {
                    Ok(native) => native,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                if ready_tx.send(Ok(())).is_err() {
                    return;
                }
                loop {
                    Worker::pump();
                    match receiver.recv_timeout(Duration::from_millis(10)) {
                        Ok(job) => job(&native),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .map_err(|_| PlatformError::PasteboardReadFailed)?;
        if let Ok(Ok(())) = ready_rx.recv() {
            Ok(Self {
                sender: Some(sender),
                worker: Some(worker),
            })
        } else {
            drop(sender);
            let _ = worker.join();
            Err(PlatformError::PasteboardReadFailed)
        }
    }

    fn request<T: Send + 'static>(
        &self,
        job: impl FnOnce(&Worker) -> PlatformResult<T> + Send + 'static,
    ) -> PlatformResult<T> {
        let (reply, receive) = mpsc::sync_channel(1);
        self.sender
            .as_ref()
            .ok_or(PlatformError::PasteboardReadFailed)?
            .send(Box::new(move |worker| {
                let _ = reply.send(job(worker));
            }))
            .map_err(|_| PlatformError::PasteboardReadFailed)?;
        receive
            .recv()
            .map_err(|_| PlatformError::PasteboardReadFailed)?
    }
}

impl Drop for WindowsPasteboard {
    fn drop(&mut self) {
        drop(self.sender.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Pasteboard for WindowsPasteboard {
    fn change_count(&self) -> PlatformResult<i64> {
        self.request(|_| Ok(sequence().into()))
    }
    fn read(&self) -> PlatformResult<PasteboardSnapshot> {
        self.request(Worker::read)
    }
    fn write_text(&self, text: &str) -> PlatformResult<()> {
        let bytes = formats::encode_utf16(text)?;
        self.request(move |worker| worker.replace(vec![(CF_UNICODETEXT, bytes)]))
    }
    fn write_png(&self, bytes: &[u8]) -> PlatformResult<()> {
        if !formats::validate_png(bytes) {
            return Err(PlatformError::PasteboardWriteFailed);
        }
        let bytes = bytes.to_vec();
        self.request(move |worker| {
            let dib = formats::png_to_dibv5(&bytes)?;
            worker.replace(vec![(worker.png, bytes), (CF_DIBV5, dib)])
        })
    }
    fn write_file_urls(&self, paths: &[String]) -> PlatformResult<()> {
        // The shared API historically names this field file_urls, while the
        // transfer pipeline consumes local paths. Accept either at this boundary.
        let paths = paths
            .iter()
            .map(|value| {
                if value.starts_with("file:") {
                    url::Url::parse(value)
                        .ok()
                        .and_then(|url| url.to_file_path().ok())
                        .and_then(|path| path.into_os_string().into_string().ok())
                        .ok_or(PlatformError::PasteboardFileUnavailable)
                } else {
                    Ok(value.clone())
                }
            })
            .collect::<PlatformResult<Vec<_>>>()?;
        let bytes = formats::encode_hdrop(&paths)?;
        if paths
            .iter()
            .any(|path| !std::path::Path::new(path).exists())
        {
            return Err(PlatformError::PasteboardFileUnavailable);
        }
        self.request(move |worker| worker.replace(vec![(CF_HDROP, bytes)]))
    }
    fn clear_for_remote_file(&self, marker: &str) -> PlatformResult<()> {
        let bytes = marker_bytes(marker)?;
        self.request(move |worker| worker.replace(vec![(worker.marker, bytes)]))
    }
    fn write_items(&self, items: &[PasteboardItem]) -> PlatformResult<()> {
        if items.len() != 1 {
            return Err(PlatformError::Unsupported {
                operation: "Windows clipboard multiple item representations",
            });
        }
        if items[0].representations.len() > 16
            || items[0]
                .representations
                .iter()
                .try_fold(0usize, |sum, rep| sum.checked_add(rep.data.len()))
                .is_none_or(|size| size > MAX_FORMAT_BYTES)
        {
            return Err(PlatformError::PasteboardWriteFailed);
        }
        let items = items.to_vec();
        self.request(move |worker| {
            let mut output = Vec::new();
            for rep in &items[0].representations {
                let (format, bytes) = match rep.type_name.as_str() {
                    "text/plain" | "public.utf8-plain-text" => (
                        CF_UNICODETEXT,
                        formats::encode_utf16(
                            std::str::from_utf8(&rep.data)
                                .map_err(|_| PlatformError::PasteboardWriteFailed)?,
                        )?,
                    ),
                    "image/png" | "public.png" => {
                        output.push((CF_DIBV5, formats::png_to_dibv5(&rep.data)?));
                        (worker.png, rep.data.clone())
                    }
                    "text/html" | "public.html" => (worker.html, formats::encode_html(&rep.data)?),
                    "text/rtf" | "application/rtf" | "public.rtf" => {
                        (worker.rtf, terminated_bytes(&rep.data)?)
                    }
                    REMOTE_ORIGIN_MARKER_TYPE => (
                        worker.marker,
                        marker_bytes(
                            std::str::from_utf8(&rep.data)
                                .map_err(|_| PlatformError::PasteboardWriteFailed)?,
                        )?,
                    ),
                    _ => {
                        return Err(PlatformError::Unsupported {
                            operation: "Windows clipboard representation",
                        });
                    }
                };
                output.push((format, bytes));
                formats::validate_native_entries(&output)?;
            }
            if output.is_empty() {
                return Err(PlatformError::PasteboardWriteFailed);
            }
            worker.replace(output)
        })
    }
    fn read_remote_origin_marker(&self) -> PlatformResult<Option<String>> {
        self.request(|worker| {
            let _opened = worker.open()?;
            worker
                .read_bytes(worker.marker, 1024)?
                .map(|bytes| {
                    let bytes = bytes.split(|byte| *byte == 0).next().unwrap_or_default();
                    String::from_utf8(bytes.to_vec())
                        .map_err(|_| PlatformError::PasteboardReadFailed)
                })
                .transpose()
        })
    }
    fn write_remote_origin_marker(&self, marker: &str) -> PlatformResult<()> {
        let bytes = marker_bytes(marker)?;
        self.request(move |worker| {
            let opened = worker.open()?;
            // Do not tag a subsequent local copy as remote if the user copied
            // between write_content and write_marker.
            // SAFETY: clipboard owner query has no pointer inputs.
            if worker.last_write.get() != Some(sequence())
                || unsafe { GetClipboardOwner() } != worker.hwnd
            {
                return Err(PlatformError::PasteboardWriteFailed);
            }
            let block = GlobalBlock::new(&bytes)?;
            block.publish(worker.marker)?;
            drop(opened);
            worker.last_write.set(Some(sequence()));
            Ok(())
        })
    }
}

fn terminated_bytes(bytes: &[u8]) -> PlatformResult<Vec<u8>> {
    if bytes.len() >= MAX_FORMAT_BYTES || bytes.contains(&0) {
        return Err(PlatformError::PasteboardWriteFailed);
    }
    let mut result = bytes.to_vec();
    result.push(0);
    Ok(result)
}
fn marker_bytes(marker: &str) -> PlatformResult<Vec<u8>> {
    if marker.is_empty()
        || marker.len() > 512
        || !marker.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(PlatformError::PasteboardWriteFailed);
    }
    terminated_bytes(marker.as_bytes())
}
fn sequence() -> u32 {
    // SAFETY: the Win32 sequence query has no arguments or owned resources.
    unsafe { GetClipboardSequenceNumber() }
}

struct Worker {
    hwnd: HWND,
    png: u32,
    html: u32,
    rtf: u32,
    marker: u32,
    virtual_files: u32,
    virtual_files_ansi: u32,
    last_write: Cell<Option<u32>>,
}
impl Worker {
    fn new() -> PlatformResult<Self> {
        let register = |name: &str| {
            let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
            // SAFETY: the format name is NUL terminated and live for the call.
            let format = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
            if format == 0 {
                Err(PlatformError::PasteboardReadFailed)
            } else {
                Ok(format)
            }
        };
        // Register before creating the HWND so a later failure cannot leak it.
        let png = register("PNG")?;
        let html = register("HTML Format")?;
        let rtf = register("Rich Text Format")?;
        let marker = register(REMOTE_ORIGIN_MARKER_TYPE)?;
        let virtual_files = register("FileGroupDescriptorW")?;
        let virtual_files_ansi = register("FileGroupDescriptor")?;
        let class: Vec<u16> = "STATIC".encode_utf16().chain([0]).collect();
        // SAFETY: STATIC is a system class; message-only parent and null optional
        // parameters are documented. Window creation/destruction stay on this thread.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if hwnd.is_null() {
            return Err(PlatformError::PasteboardReadFailed);
        }
        Ok(Self {
            hwnd,
            png,
            html,
            rtf,
            marker,
            virtual_files,
            virtual_files_ansi,
            last_write: Cell::new(None),
        })
    }
    fn pump() {
        let mut message = MSG::default();
        // SAFETY: MSG is initialized, valid, and only this thread dispatches its
        // own window messages; no callback refers to a Rust stack context.
        unsafe {
            while PeekMessageW(&raw mut message, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&raw const message);
                DispatchMessageW(&raw const message);
            }
        }
    }
    fn open(&self) -> PlatformResult<OpenedClipboard> {
        for _ in 0..5 {
            // SAFETY: HWND is live and owned by this worker thread.
            if unsafe { OpenClipboard(self.hwnd) } != 0 {
                return Ok(OpenedClipboard);
            }
            Self::pump();
            thread::sleep(Duration::from_millis(10));
        }
        Err(PlatformError::PasteboardReadFailed)
    }
    /// Must be called while `OpenedClipboard` is alive. Only whitelisted formats
    /// known to contain HGLOBAL memory may be passed here.
    fn read_bytes(&self, format: u32, maximum: usize) -> PlatformResult<Option<Vec<u8>>> {
        debug_assert!(
            [
                CF_UNICODETEXT,
                CF_HDROP,
                CF_DIB,
                CF_DIBV5,
                self.png,
                self.html,
                self.rtf,
                self.marker
            ]
            .contains(&format)
        );
        // SAFETY: clipboard is open; these documented formats use HGLOBAL. Size
        // is checked before locking, only that allocation is copied, and it is
        // unlocked before clipboard closes. We never free clipboard-owned data.
        unsafe {
            if IsClipboardFormatAvailable(format) == 0 {
                return Ok(None);
            }
            let handle = GetClipboardData(format);
            if handle.is_null() {
                return Err(PlatformError::PasteboardReadFailed);
            }
            let size = GlobalSize(handle);
            if size == 0 || size > maximum {
                return Err(PlatformError::PasteboardReadFailed);
            }
            let data = GlobalLock(handle);
            if data.is_null() {
                return Err(PlatformError::PasteboardReadFailed);
            }
            let bytes = std::slice::from_raw_parts(data.cast::<u8>(), size).to_vec();
            GlobalUnlock(handle);
            Ok(Some(bytes))
        }
    }
    fn read(&self) -> PlatformResult<PasteboardSnapshot> {
        let opened = self.open()?;
        let change_count = sequence();
        let mut snapshot = PasteboardSnapshot {
            change_count: change_count.into(),
            ..Default::default()
        };
        if let Some(bytes) = self.read_bytes(CF_HDROP, MAX_FORMAT_BYTES)? {
            if sequence() != change_count {
                return Err(PlatformError::PasteboardReadFailed);
            }
            drop(opened);
            snapshot.file_urls = formats::decode_hdrop(&bytes)?;
            return Ok(snapshot);
        }
        // Virtual-file names must never be sent in lieu of file bytes.
        // SAFETY: availability queries only take integer format identifiers.
        if unsafe {
            IsClipboardFormatAvailable(self.virtual_files) != 0
                || IsClipboardFormatAvailable(self.virtual_files_ansi) != 0
        } {
            return Err(PlatformError::PasteboardFileUnavailable);
        }
        let mut raw = Vec::new();
        let mut remaining = MAX_FORMAT_BYTES;
        // Prefer exact PNG, then DIBV5, then DIB. Never lock a CF_BITMAP handle.
        for (format, name) in [
            (self.png, "image/png"),
            (CF_DIBV5, "image/dib"),
            (CF_DIB, "image/dib"),
        ] {
            if let Some(bytes) = self.read_bytes(format, remaining)? {
                remaining -= bytes.len();
                raw.push((name, bytes));
                break;
            }
        }
        // SAFETY: CF_BITMAP is queried but not retrieved or treated as HGLOBAL.
        if raw.is_empty() && unsafe { IsClipboardFormatAvailable(2) } != 0 {
            return Err(PlatformError::Unsupported {
                operation: "Windows CF_BITMAP-only clipboard image conversion",
            });
        }
        for (format, name) in [
            (CF_UNICODETEXT, "text/plain"),
            (self.html, "text/html"),
            (self.rtf, "text/rtf"),
        ] {
            if let Some(bytes) = self.read_bytes(format, remaining)? {
                remaining -= bytes.len();
                raw.push((name, bytes));
            }
        }
        // Delayed rendering can update the sequence even while open. All raw
        // bytes must come from one copy operation. Release the system clipboard
        // before CPU-heavy PNG conversion or any other format decoding.
        if sequence() != change_count {
            return Err(PlatformError::PasteboardReadFailed);
        }
        drop(opened);
        let mut representations = Vec::new();
        let mut output_remaining = MAX_FORMAT_BYTES;
        for (name, bytes) in raw {
            let (name, data) = match name {
                "text/plain" => {
                    let text = formats::decode_utf16(&bytes)?;
                    snapshot.text = Some(text.clone());
                    (name, text.into_bytes())
                }
                "image/png" => {
                    if !formats::validate_png(&bytes) {
                        return Err(PlatformError::PasteboardReadFailed);
                    }
                    snapshot.png = Some(bytes.clone());
                    (name, bytes)
                }
                "image/dib" => {
                    let png = formats::dib_to_png(&bytes)?;
                    snapshot.png = Some(png.clone());
                    ("image/png", png)
                }
                "text/html" => (name, formats::decode_html(&bytes)?),
                _ => (
                    name,
                    bytes
                        .split(|byte| *byte == 0)
                        .next()
                        .unwrap_or_default()
                        .to_vec(),
                ),
            };
            output_remaining = output_remaining
                .checked_sub(data.len())
                .ok_or(PlatformError::PasteboardReadFailed)?;
            representations.push(PasteboardRepresentation {
                type_name: name.into(),
                data,
            });
        }
        if !representations.is_empty() {
            snapshot.items.push(PasteboardItem { representations });
        }
        Ok(snapshot)
    }
    fn replace(&self, entries: Vec<(u32, Vec<u8>)>) -> PlatformResult<()> {
        formats::validate_native_entries(&entries)?;
        // Allocate everything before EmptyClipboard, so allocation failure
        // cannot destroy the user's previous clipboard.
        let blocks = entries
            .into_iter()
            .map(|(format, bytes)| GlobalBlock::new(&bytes).map(|block| (format, block)))
            .collect::<PlatformResult<Vec<_>>>()?;
        let opened = self
            .open()
            .map_err(|_| PlatformError::PasteboardWriteFailed)?;
        self.last_write.set(None);
        // SAFETY: clipboard is opened using a non-null, live owner HWND.
        if unsafe { EmptyClipboard() } == 0 {
            return Err(PlatformError::PasteboardWriteFailed);
        }
        for (format, block) in blocks {
            block.publish(format)?;
        }
        drop(opened);
        self.last_write.set(Some(sequence()));
        Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        // SAFETY: the HWND was created and is destroyed on this same thread.
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}
struct OpenedClipboard;
impl Drop for OpenedClipboard {
    fn drop(&mut self) {
        // SAFETY: this guard exists only after OpenClipboard succeeds and stays
        // on the opening worker thread.
        unsafe {
            CloseClipboard();
        }
    }
}
struct GlobalBlock(HGLOBAL);
impl GlobalBlock {
    fn new(bytes: &[u8]) -> PlatformResult<Self> {
        if bytes.is_empty() || bytes.len() > MAX_FORMAT_BYTES {
            return Err(PlatformError::PasteboardWriteFailed);
        }
        // SAFETY: allocate enough movable global memory for the copy, check
        // every handle/pointer, and unlock before publishing to Win32.
        unsafe {
            let handle = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
            if handle.is_null() {
                return Err(PlatformError::PasteboardWriteFailed);
            }
            let block = Self(handle);
            let data = GlobalLock(handle);
            if data.is_null() {
                return Err(PlatformError::PasteboardWriteFailed);
            }
            ptr::copy_nonoverlapping(bytes.as_ptr(), data.cast::<u8>(), bytes.len());
            GlobalUnlock(handle);
            Ok(block)
        }
    }
    fn publish(mut self, format: u32) -> PlatformResult<()> {
        // SAFETY: clipboard is open, and this is a movable HGLOBAL allocation
        // for a whitelisted format. Successful publication transfers ownership.
        if unsafe { SetClipboardData(format, self.0) }.is_null() {
            return Err(PlatformError::PasteboardWriteFailed);
        }
        self.0 = ptr::null_mut();
        Ok(())
    }
}
impl Drop for GlobalBlock {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: a non-null handle here was not transferred to clipboard.
            unsafe {
                GlobalFree(self.0);
            }
        }
    }
}
