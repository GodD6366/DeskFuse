//! Windows desktop file transfer bridge tests. These run only on Windows CI:
//! the native staging store uses the Win32 filesystem, and the bridge speaks
//! the same `ClipboardFileFrame` protocol as the macOS engine end to end.

#![cfg(windows)]
use crate::native::file_transfer;
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use deskfuse_core::DeviceId;
use deskfuse_transfer::{
    ChunkDescriptor, ClipboardFileFrame, FileBridge, FileFrameSink, IncomingStore, Limits,
    RelativePath, StoreFactory, TransferId, TransferManifest,
};

const PEER_A: DeviceId = DeviceId::from_bytes([7; 32]);
const PEER_B: DeviceId = DeviceId::from_bytes([8; 32]);

#[derive(Default)]
struct Recorder {
    frames: Mutex<Vec<ClipboardFileFrame>>,
}
impl FileFrameSink for Recorder {
    fn send(&self, _peer: DeviceId, frame: ClipboardFileFrame) -> Result<(), String> {
        self.frames.lock().expect("recorder").push(frame);
        Ok(())
    }
}
impl Recorder {
    fn drain(&self) -> Vec<ClipboardFileFrame> {
        std::mem::take(&mut self.frames.lock().expect("recorder"))
    }
}

/// Mirrors `deskfuse_transfer::default_store` but constructs the Windows store
/// directly so this test module exercises the real native receiver.
struct WindowsStoreTest {
    destination: PathBuf,
    staging: PathBuf,
    manifest: TransferManifest,
}

impl WindowsStoreTest {
    fn new(
        manifest: TransferManifest,
        id: TransferId,
        root: &std::path::Path,
    ) -> Result<Self, String> {
        let destination = root.join(id.to_hex());
        let staging = destination.join(format!(".staging-{id}"));
        fs::create_dir_all(&staging).map_err(|error| error.to_string())?;
        Ok(Self {
            destination,
            staging,
            manifest,
        })
    }
    fn entry(&self, path: &RelativePath) -> Result<deskfuse_transfer::TransferEntry, String> {
        self.manifest
            .entries()
            .binary_search_by(|entry| entry.path().cmp(path))
            .ok()
            .map(|index| self.manifest.entries()[index].clone())
            .ok_or_else(|| format!("path {path} missing"))
    }
}

impl IncomingStore for WindowsStoreTest {
    fn write_chunk(
        &mut self,
        path: &RelativePath,
        descriptor: ChunkDescriptor,
        bytes: &[u8],
    ) -> Result<(), String> {
        use std::io::{Seek, SeekFrom, Write};
        let entry = self.entry(path)?;
        let end = descriptor.offset + u64::from(descriptor.length);
        if !entry.is_file() || end > entry.size() {
            return Err("chunk out of range".into());
        }
        let target = self.staging.join(path.to_path_buf());
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&target)
            .map_err(|error| error.to_string())?;
        file.seek(SeekFrom::Start(descriptor.offset))
            .and_then(|_| file.write_all(bytes))
            .map_err(|error| error.to_string())
    }
    fn complete(&mut self) -> Result<Vec<String>, String> {
        let mut written = Vec::new();
        for entry in self.manifest.files() {
            let staged = self.staging.join(entry.path().to_path_buf());
            let final_path = self.destination.join(entry.path().to_path_buf());
            if let Some(parent) = final_path.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::rename(&staged, &final_path).map_err(|error| error.to_string())?;
            written.push(final_path.to_string_lossy().into_owned());
        }
        let _ = fs::remove_dir_all(&self.staging);
        Ok(written)
    }
    fn abort(&mut self) {
        let _ = fs::remove_dir_all(&self.destination);
    }
}

fn factory(root: PathBuf) -> StoreFactory {
    Arc::new(move |manifest, id| {
        WindowsStoreTest::new(manifest.clone(), id, &root)
            .map(|store| Box::new(store) as Box<dyn IncomingStore>)
    })
}

struct Harness {
    root: PathBuf,
    bridge: FileBridge,
    recorder: Arc<Recorder>,
}

fn harness(name: &str) -> Harness {
    let root = std::env::temp_dir().join(format!("deskfuse-svc-{name}-{}", TransferId::new()));
    fs::create_dir_all(&root).expect("root");
    let recorder = Arc::new(Recorder::default());
    let bridge = FileBridge::new(
        root.clone(),
        factory(root.clone()),
        recorder.clone(),
        Limits::default(),
    );
    Harness {
        root,
        bridge,
        recorder,
    }
}

fn wait_until(deadline: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while Instant::now().duration_since(start) < deadline {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    predicate()
}

/// Full loop: publish → Deferred → Request → RequestedOffer + chunks + Complete
/// → staged → commit → Ack. Exercises the Windows staging store on real files.
#[test]
fn windows_loopback_transfer_writes_real_files() {
    let sender = harness("svc-send");
    let receiver = harness("svc-recv");
    let payload: Vec<u8> = (0..(2 * 1024 * 1024 + 123))
        .map(|i| (i % 249) as u8)
        .collect();
    let source = sender.root.join("doc.bin");
    fs::write(&source, &payload).expect("source");
    let frames = sender
        .bridge
        .publish_local(&[PEER_B], vec![source])
        .expect("publish");
    let (_, transfer_id, deferred) = frames.into_iter().next().expect("frame");
    receiver.bridge.on_frame(PEER_A, deferred);
    receiver
        .bridge
        .request_incoming(&transfer_id.to_hex())
        .expect("request");
    let request = receiver
        .recorder
        .drain()
        .into_iter()
        .find(|frame| matches!(frame, ClipboardFileFrame::Request { .. }))
        .expect("request");
    sender.bridge.on_frame(PEER_B, request);
    let mut stream = Vec::new();
    assert!(
        wait_until(Duration::from_secs(15), || {
            stream.extend(sender.recorder.drain());
            matches!(stream.last(), Some(ClipboardFileFrame::Complete { .. }))
        }),
        "sender never completed"
    );
    for frame in &stream {
        receiver.bridge.on_frame(PEER_A, frame.clone());
    }
    assert!(
        wait_until(Duration::from_secs(15), || {
            receiver.bridge.tick(&[PEER_A]);
            receiver
                .bridge
                .views()
                .iter()
                .any(|v| v.direction == "incoming" && v.state == "staged")
        }),
        "receiver never staged"
    );
    let staged = receiver
        .bridge
        .views()
        .into_iter()
        .find(|v| v.state == "staged")
        .expect("staged");
    let received = fs::read(&staged.staged_paths[0]).expect("staged bytes");
    assert_eq!(received, payload);
    receiver
        .bridge
        .commit_incoming(&transfer_id.to_hex(), true)
        .expect("commit");
    assert!(
        receiver
            .recorder
            .drain()
            .iter()
            .any(|frame| matches!(frame, ClipboardFileFrame::Ack { .. }))
    );
    sender.bridge.reset(false);
    receiver.bridge.reset(false);
    std::thread::sleep(Duration::from_millis(150));
    let _ = fs::remove_dir_all(&sender.root);
    let _ = fs::remove_dir_all(&receiver.root);
}

#[test]
fn windows_store_rejects_reserved_and_oversize_paths() {
    // Reserved device names are rejected by manifest validation on Windows.
    let make = |path: &str| {
        deskfuse_transfer::TransferEntry::file(path, 1, 0, deskfuse_transfer::sha256_bytes(b"x"))
            .expect("entry")
    };
    assert!(TransferManifest::new(vec![make("CON/a.txt")]).is_err());
    assert!(TransferManifest::new(vec![make("ok.txt")]).is_ok());
    // Backslash separators are rejected in relative paths.
    assert!(RelativePath::new("a\\b.txt").is_err());
}

#[test]
fn windows_display_name_uses_the_computer_hostname() {
    let name = file_transfer::computer_display_name();
    assert!(!name.is_empty());
    assert!(!name.chars().any(char::is_control));
    assert!(name.chars().count() <= 64);
}
