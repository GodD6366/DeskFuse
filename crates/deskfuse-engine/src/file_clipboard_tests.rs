//! Receiver integration tests using the production manifest, framing, staging,
//! hash checks, origin marker, and clipboard write path.

use super::*;
use deskfuse_platform::MemoryPasteboard;
use deskfuse_platform_macos::MacOSPasteboard;

struct Fixture {
    source: PathBuf,
    received: PathBuf,
    plan: ClipboardFilePlan,
    retain: bool,
}

impl Fixture {
    fn new(retain: bool) -> Self {
        let transfer_id = TransferId::new();
        let source = std::env::temp_dir().join(format!("deskfuse-file-paste-{transfer_id}"));
        let folder = source.join("DeskFuse 文件夹");
        std::fs::create_dir_all(folder.join("空目录")).expect("fixture directories");
        std::fs::write(
            folder.join("中文 data.bin"),
            vec![0x5a; 3 * 1024 * 1024 + 17],
        )
        .expect("multi-chunk fixture");
        let empty = source.join("DeskFuse 零字节.txt");
        std::fs::write(&empty, []).expect("empty fixture");
        let plan = ClipboardFilePlan::from_paths_with_transfer_id(&[folder, empty], transfer_id)
            .expect("file plan");
        let received = source
            .canonicalize()
            .unwrap()
            .join("private-test-cache")
            .join(transfer_id.to_string());
        Self {
            source,
            received,
            plan,
            retain,
        }
    }

    fn frames(&self, corrupt: bool) -> Vec<ClipboardFileFrame> {
        let id = self.plan.transfer_id();
        let mut frames = vec![ClipboardFileFrame::Offer {
            transfer_id: id,
            manifest: self.plan.manifest().clone(),
        }];
        for source in self.plan.sources() {
            for descriptor in source.chunks() {
                let mut bytes = read_chunk(source.source_path(), *descriptor).expect("read chunk");
                if corrupt && !bytes.is_empty() {
                    bytes[0] ^= 1;
                }
                frames.push(ClipboardFileFrame::Chunk {
                    transfer_id: id,
                    path: source.relative_path().clone(),
                    descriptor: *descriptor,
                    bytes,
                });
            }
        }
        frames.push(ClipboardFileFrame::Complete { transfer_id: id });
        frames
    }

    fn verify_received(&self, pasteboard: &impl Pasteboard) {
        let snapshot = pasteboard.read().expect("received pasteboard");
        assert_eq!(
            snapshot.file_urls.len(),
            2,
            "both top-level items remain after marking"
        );
        assert!(
            snapshot.text.is_none(),
            "must not publish the filename as text"
        );
        for source in self.plan.sources() {
            let target = self.received.join(source.relative_path().as_str());
            assert_eq!(
                std::fs::read(source.source_path()).expect("source bytes"),
                std::fs::read(target).expect("received bytes")
            );
        }
        assert!(self.received.join("DeskFuse 文件夹/空目录").is_dir());
        for path in &snapshot.file_urls {
            assert!(PathBuf::from(path).exists());
            assert!(PathBuf::from(path).starts_with(&self.received));
        }
        assert_eq!(
            pasteboard
                .read_remote_origin_marker()
                .expect("origin marker"),
            Some(format!(
                "deskfuse-clipboard-file-v1:{}",
                self.plan.transfer_id()
            ))
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if !self.retain {
            let _ = std::fs::remove_dir_all(&self.source);
            let _ = std::fs::remove_dir_all(&self.received);
        }
    }
}

struct ReceiveHarness {
    last: Option<i64>,
    incoming: HashMap<(DeviceId, TransferId), IncomingFileClipboard>,
    cache: TransferCache,
    guard: ClipboardLoopGuard,
    history: ClipboardHistoryStore,
    pending: AtomicBool,
    requested: AtomicBool,
    target: AtomicI32,
    sender: ReceiveEventSender,
    events: TokioReceiver<ReceiveEvent>,
    ui: UnboundedSender<UiEvent>,
    ui_events: UnboundedReceiver<UiEvent>,
    diagnostics: Diagnostics,
}
impl ReceiveHarness {
    fn new(fixture: &Fixture) -> Self {
        let (sender, events) = receive_event_channel();
        let (ui, ui_events) = unbounded();
        Self {
            last: None,
            incoming: HashMap::new(),
            cache: TransferCache::new(
                fixture.received.parent().unwrap().to_path_buf(),
                1024 * 1024 * 1024,
            )
            .unwrap(),
            guard: ClipboardLoopGuard::default(),
            history: ClipboardHistoryStore::default(),
            pending: AtomicBool::new(false),
            requested: AtomicBool::new(false),
            target: AtomicI32::new(0),
            sender,
            events,
            ui,
            ui_events,
            diagnostics: Diagnostics::new(32),
        }
    }
    fn feed(&mut self, pasteboard: &impl Pasteboard, frame: ClipboardFileFrame) {
        let payload =
            ClipboardPayload::new(CLIPBOARD_FILE_MIME, frame.encode().unwrap(), 1).unwrap();
        let result = apply_remote_clipboard(
            pasteboard,
            &payload,
            &mut RemoteClipboardState {
                last_change_count: &mut self.last,
                incoming_files: &mut self.incoming,
                receive_events: &self.sender,
                cache: &mut self.cache,
                loop_guard: &mut self.guard,
                history: &mut self.history,
                pending_incoming_file: &self.pending,
                paste_requested: &self.requested,
                paste_target_pid: &self.target,
            },
            &self.ui,
            &self.diagnostics,
            DeviceId::from_bytes([0x5a; 32]),
        );
        assert!(
            result.is_none(),
            "network frames never synchronously commit or acknowledge files"
        );
    }
    async fn finish(&mut self, pasteboard: &impl Pasteboard) -> Vec<(DeviceId, TransferId)> {
        tokio::time::timeout(Duration::from_secs(10), async {
            let mut acknowledgements = Vec::new();
            while !self.incoming.is_empty() {
                let event = self.events.recv().await.expect("disk completion");
                let result = handle_receive_event(
                    event,
                    pasteboard,
                    &mut RemoteClipboardState {
                        last_change_count: &mut self.last,
                        incoming_files: &mut self.incoming,
                        receive_events: &self.sender,
                        cache: &mut self.cache,
                        loop_guard: &mut self.guard,
                        history: &mut self.history,
                        pending_incoming_file: &self.pending,
                        paste_requested: &self.requested,
                        paste_target_pid: &self.target,
                    },
                    &self.ui,
                    &self.diagnostics,
                );
                if let Some(result) = result {
                    acknowledgements.push(result);
                }
            }
            acknowledgements
        })
        .await
        .expect("receiver completion timeout")
    }
}
async fn receive(pasteboard: &impl Pasteboard, fixture: &Fixture, corrupt: bool) {
    let mut harness = ReceiveHarness::new(fixture);
    for frame in fixture.frames(corrupt) {
        harness.feed(pasteboard, frame);
    }
    let acknowledgements = harness.finish(pasteboard).await;
    assert_eq!(acknowledgements.len(), usize::from(!corrupt));
    assert!(!harness.pending.load(Ordering::Acquire));
    assert_eq!(harness.cache.snapshot().unwrap().active_reserved_bytes, 0);
    if corrupt {
        assert!(
            harness
                .diagnostics
                .snapshot()
                .iter()
                .any(|record| record.event == "file_apply_failed")
        );
    } else {
        fixture.verify_received(pasteboard);
    }
}

#[test]
fn first_file_receive_creates_missing_private_cache_parents() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("deskfuse-cache-test-{}", TransferId::new()));
    let destination = root.join("app/clipboard/transfer");
    let receiver = clipboard_file_receiver(&destination).unwrap();
    for path in [
        &root,
        &root.join("app"),
        &root.join("app/clipboard"),
        &destination,
    ] {
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    drop(receiver);
    std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn file_clipboard_transfers_folders_multiple_chunks_empty_files_and_marks_file_urls() {
    let fixture = Fixture::new(false);
    receive(&MemoryPasteboard::new(), &fixture, false).await;
}
#[tokio::test]
async fn corrupt_file_clipboard_never_publishes_paths_or_acknowledges_success() {
    let fixture = Fixture::new(false);
    let pasteboard = MemoryPasteboard::new();
    receive(&pasteboard, &fixture, true).await;
    assert!(pasteboard.read().unwrap().file_urls.is_empty());
    assert!(!fixture.received.exists());
}
#[tokio::test]
#[ignore = "writes the general macOS pasteboard and keeps fixtures for a manual Finder Cmd-V check"]
async fn native_file_clipboard_for_finder_paste() {
    let fixture = Fixture::new(true);
    receive(&MacOSPasteboard::new(), &fixture, false).await;
    println!("Finder paste source fixture: {}", fixture.source.display());
    println!(
        "Finder paste received fixture: {}",
        fixture.received.display()
    );
}
#[tokio::test]
async fn missing_file_chunks_cannot_paste_stale_text_and_surface_a_failed_transfer() {
    let fixture = Fixture::new(false);
    let pasteboard = MemoryPasteboard::new();
    pasteboard
        .write_text("previous unrelated clipboard text")
        .unwrap();
    let mut harness = ReceiveHarness::new(&fixture);
    harness.feed(&pasteboard, fixture.frames(false).remove(0));
    harness.feed(
        &pasteboard,
        ClipboardFileFrame::Complete {
            transfer_id: fixture.plan.transfer_id(),
        },
    );
    assert!(pasteboard.read().unwrap().text.is_none());
    assert!(pasteboard.read().unwrap().file_urls.is_empty());
    assert!(harness.finish(&pasteboard).await.is_empty());
    assert!(!harness.pending.load(Ordering::Acquire));
    assert!(
        std::iter::from_fn(|| harness.ui_events.try_recv().ok()).any(|event| matches!(
            event,
            UiEvent::TransferStatus {
                state: TransferState::Failed,
                ..
            }
        ))
    );
}
#[tokio::test]
async fn superseded_clipboard_cannot_be_overwritten_or_replayed_by_late_completion() {
    let fixture = Fixture::new(false);
    let pasteboard = MemoryPasteboard::new();
    let mut harness = ReceiveHarness::new(&fixture);
    for frame in fixture.frames(false) {
        if matches!(frame, ClipboardFileFrame::Complete { .. }) {
            harness.requested.store(true, Ordering::Release);
            harness.target.store(42, Ordering::Release);
            pasteboard.write_text("new local clipboard").unwrap();
        }
        harness.feed(&pasteboard, frame);
    }
    assert_eq!(
        harness.finish(&pasteboard).await,
        vec![(DeviceId::from_bytes([0x5a; 32]), fixture.plan.transfer_id())]
    );
    assert_eq!(
        pasteboard.read().unwrap().text.as_deref(),
        Some("new local clipboard")
    );
    assert_eq!(
        harness.cache.snapshot().unwrap().used_bytes,
        fixture.plan.manifest().total_bytes()
    );
    for source in fixture.plan.sources() {
        assert_eq!(
            std::fs::read(source.source_path()).unwrap(),
            std::fs::read(fixture.received.join(source.relative_path().as_str())).unwrap()
        );
    }
    assert_eq!(harness.cache.snapshot().unwrap().active_reserved_bytes, 0);
    assert!(
        !std::iter::from_fn(|| harness.ui_events.try_recv().ok())
            .any(|event| matches!(event, UiEvent::PasteReady { .. }))
    );
}
#[test]
fn receiver_rejects_over_quota_manifest_before_creating_staging() {
    let fixture = Fixture::new(false);
    let mut harness = ReceiveHarness::new(&fixture);
    harness.cache.set_limit(1024 * 1024).unwrap();
    let error = apply_remote_file_clipboard(
        &MemoryPasteboard::new(),
        &fixture.frames(false)[0].encode().unwrap(),
        &mut harness.incoming,
        &mut harness.cache,
        DeviceId::from_bytes([63; 32]),
        &harness.sender,
        &harness.ui,
    )
    .unwrap_err();
    assert!(error.contains("缓存空间不足"));
    assert!(harness.incoming.is_empty());
    assert!(!fixture.received.exists());
    assert_eq!(harness.cache.snapshot().unwrap().active_reserved_bytes, 0);
}
#[tokio::test]
async fn cleanup_invalidates_only_history_and_pasteboard_references_inside_removed_cache() {
    let fixture = Fixture::new(false);
    let pasteboard = MemoryPasteboard::new();
    receive(&pasteboard, &fixture, false).await;
    let (mut history, mut last) = (ClipboardHistoryStore::default(), None);
    let (sender, _) = unbounded();
    // 临时目录中的文件路径不会进入历史库，这里直接操作内存历史，
    // 让用例聚焦在缓存清理对历史/粘贴板引用的失效处理上。
    history.insert(pasteboard.read().unwrap(), "received");
    let original = fixture.plan.sources()[0]
        .source_path()
        .to_string_lossy()
        .into_owned();
    history.insert(
        PasteboardSnapshot {
            file_urls: vec![original.clone()],
            ..Default::default()
        },
        "source",
    );
    let mut cache = TransferCache::new(
        fixture.received.parent().unwrap().to_path_buf(),
        1024 * 1024 * 1024,
    )
    .unwrap();
    let cleanup = cache.clear_completed().unwrap();
    file_policy::apply_cleanup(cleanup, &mut history, &pasteboard, &mut last, &sender);
    assert_eq!(history.items.len(), 1);
    assert_eq!(history.items[0].1.file_urls, vec![original.clone()]);
    assert!(PathBuf::from(original).exists());
    assert!(pasteboard.read().unwrap().file_urls.is_empty());
}
struct CommitGateLease(Arc<ReceiveTestGate>);
impl Drop for CommitGateLease {
    fn drop(&mut self) {
        self.0.release();
    }
}
async fn wait_for_commit_gate(gate: &ReceiveTestGate) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !gate.entered.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("disk worker should reach commit");
}
#[tokio::test(flavor = "current_thread")]
async fn slow_commit_keeps_heartbeat_and_cancel_responsive_and_reserves_until_worker_exit() {
    let fixture = Fixture::new(false);
    let pasteboard = MemoryPasteboard::new();
    let mut harness = ReceiveHarness::new(&fixture);
    let gate = Arc::new(ReceiveTestGate::default());
    let _release_on_panic = CommitGateLease(gate.clone());
    harness.sender.gate = Some(gate.clone());
    let total = fixture.plan.manifest().total_bytes();
    harness.cache.set_limit(total).unwrap();
    for frame in fixture.frames(false) {
        harness.feed(&pasteboard, frame);
    }
    wait_for_commit_gate(&gate).await;
    // A single-thread Tokio timer continues while the disk thread is paused.
    let mut heartbeat = tokio::time::interval(Duration::from_millis(5));
    for _ in 0..4 {
        heartbeat.tick().await;
    }
    let key = (DeviceId::from_bytes([0x5a; 32]), fixture.plan.transfer_id());
    harness.incoming.get_mut(&key).unwrap().cancel();
    assert_eq!(
        harness.cache.snapshot().unwrap().active_reserved_bytes,
        total
    );
    assert!(harness.cache.reserve(TransferId::new(), 1).is_err());
    assert_eq!(harness.cache.clear_completed().unwrap().removed_bytes, 0);
    assert!(harness.incoming.contains_key(&key));
    gate.release();
    assert!(harness.finish(&pasteboard).await.is_empty());
    assert_eq!(harness.cache.snapshot().unwrap().active_reserved_bytes, 0);
    assert_eq!(harness.cache.snapshot().unwrap().used_bytes, 0);
    assert!(harness.cache.reserve(TransferId::new(), total).is_ok());
}
#[tokio::test(flavor = "current_thread")]
async fn local_copy_while_commit_is_running_is_never_overwritten_or_replayed() {
    let fixture = Fixture::new(false);
    let pasteboard = MemoryPasteboard::new();
    let mut harness = ReceiveHarness::new(&fixture);
    let gate = Arc::new(ReceiveTestGate::default());
    let _release_on_panic = CommitGateLease(gate.clone());
    harness.sender.gate = Some(gate.clone());
    for frame in fixture.frames(false) {
        harness.feed(&pasteboard, frame);
    }
    wait_for_commit_gate(&gate).await;
    harness.requested.store(true, Ordering::Release);
    harness.target.store(42, Ordering::Release);
    pasteboard.write_text("copied while committing").unwrap();
    gate.release();
    assert_eq!(
        harness.finish(&pasteboard).await,
        vec![(DeviceId::from_bytes([0x5a; 32]), fixture.plan.transfer_id())]
    );
    assert_eq!(
        pasteboard.read().unwrap().text.as_deref(),
        Some("copied while committing")
    );
    assert_eq!(harness.cache.snapshot().unwrap().active_reserved_bytes, 0);
    assert_eq!(
        harness.cache.snapshot().unwrap().used_bytes,
        fixture.plan.manifest().total_bytes()
    );
    for source in fixture.plan.sources() {
        assert_eq!(
            std::fs::read(source.source_path()).unwrap(),
            std::fs::read(fixture.received.join(source.relative_path().as_str())).unwrap()
        );
    }
    assert!(
        !std::iter::from_fn(|| harness.ui_events.try_recv().ok())
            .any(|event| matches!(event, UiEvent::PasteReady { .. }))
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_after_disk_commit_before_publication_cleans_the_committed_files() {
    let fixture = Fixture::new(false);
    let pasteboard = MemoryPasteboard::new();
    let mut harness = ReceiveHarness::new(&fixture);
    for frame in fixture.frames(false) {
        harness.feed(&pasteboard, frame);
    }
    let event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(event @ ReceiveEvent::Finished { result: Ok(()), .. }) =
                harness.events.recv().await
            {
                break event;
            }
        }
    })
    .await
    .unwrap();
    assert!(
        fixture
            .received
            .join("DeskFuse 文件夹/中文 data.bin")
            .exists()
    );
    let key = (DeviceId::from_bytes([0x5a; 32]), fixture.plan.transfer_id());
    harness.incoming.get_mut(&key).unwrap().cancel();
    assert!(
        handle_receive_event(
            event,
            &pasteboard,
            &mut RemoteClipboardState {
                last_change_count: &mut harness.last,
                incoming_files: &mut harness.incoming,
                receive_events: &harness.sender,
                cache: &mut harness.cache,
                loop_guard: &mut harness.guard,
                history: &mut harness.history,
                pending_incoming_file: &harness.pending,
                paste_requested: &harness.requested,
                paste_target_pid: &harness.target,
            },
            &harness.ui,
            &harness.diagnostics
        )
        .is_none()
    );
    assert_eq!(
        harness.cache.snapshot().unwrap().active_reserved_bytes,
        fixture.plan.manifest().total_bytes()
    );
    assert!(harness.finish(&pasteboard).await.is_empty());
    assert!(pasteboard.read().unwrap().file_urls.is_empty());
    assert_eq!(harness.cache.snapshot().unwrap().used_bytes, 0);
    assert_eq!(harness.cache.snapshot().unwrap().active_reserved_bytes, 0);
}
