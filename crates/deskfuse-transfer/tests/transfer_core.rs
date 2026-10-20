use std::fs;
use std::time::{Duration, SystemTime};

use deskfuse_transfer::{
    ChunkDescriptor, ConflictResolution, ConflictStrategy, FileReceiver, OfferOutcome,
    OfferRegistry, RelativePath, ResumeBitmap, ResumePlan, RetryPolicy, TransferEntry, TransferId,
    TransferManifest, TransferOffer, chunk_descriptors_for_data, sha256_bytes,
};
use tempfile::tempdir;

#[test]
fn directory_manifest_is_sorted_and_hashes_files() {
    let root = tempdir().expect("tempdir");
    fs::create_dir(root.path().join("z-dir")).expect("directory");
    fs::write(root.path().join("z-dir/file.txt"), b"hello").expect("file");
    fs::write(root.path().join("a.txt"), b"world").expect("file");

    let manifest = TransferManifest::from_directory(root.path()).expect("manifest");
    let paths = manifest
        .entries()
        .iter()
        .map(|entry| entry.path().to_string())
        .collect::<Vec<_>>();
    assert_eq!(paths, ["a.txt", "z-dir", "z-dir/file.txt"]);
    assert_eq!(manifest.total_bytes(), 10);
    assert_eq!(
        manifest.files().next().expect("file").hash(),
        Some(sha256_bytes(b"world"))
    );
}

#[test]
fn absolute_parent_and_windows_paths_are_rejected() {
    for path in [
        "/tmp/escape",
        "../escape",
        "a/../../escape",
        r"a\..\escape",
        r"C:\escape",
    ] {
        assert!(
            RelativePath::new(path).is_err(),
            "path unexpectedly accepted: {path}"
        );
    }
}

#[cfg(unix)]
#[test]
fn symlink_in_source_tree_is_rejected() {
    use std::os::unix::fs::symlink;
    let root = tempdir().expect("tempdir");
    let outside = tempdir().expect("outside");
    fs::write(outside.path().join("secret.txt"), b"secret").expect("secret");
    symlink(
        outside.path().join("secret.txt"),
        root.path().join("escape.txt"),
    )
    .expect("symlink");
    assert!(TransferManifest::from_directory(root.path()).is_err());
}

#[test]
fn resume_plan_round_trips_and_only_sends_missing() {
    let mut bitmap = ResumeBitmap::new(5);
    bitmap.mark_complete(1).expect("index");
    bitmap.mark_complete(3).expect("index");
    let mut plan = ResumePlan::from_bitmap(bitmap.clone());
    assert_eq!(plan.next_batch(10), [0, 2, 4]);
    plan.mark_complete(0).expect("index");
    assert_eq!(plan.missing().collect::<Vec<_>>(), [2, 4]);
    let restored = ResumeBitmap::from_bytes(5, &bitmap.to_bytes()).expect("bitmap");
    assert_eq!(restored, bitmap);
}

#[test]
fn ack_window_retries_and_frees_capacity_on_ack() {
    let mut window = deskfuse_transfer::AckWindow::new(
        1,
        RetryPolicy {
            max_retries: 2,
            retry_after: Duration::ZERO,
        },
    );
    let descriptor = ChunkDescriptor::new(0, 0, 4).expect("descriptor");
    window.register_sent(descriptor).expect("send");
    assert!(!window.can_send());
    assert_eq!(window.due_retries(SystemTime::now()), [descriptor]);
    window.record_retry(0).expect("retry");
    assert_eq!(window.retry_count(0), Some(1));
    assert_eq!(window.ack(0), Some(descriptor));
    assert!(window.can_send());
}

#[test]
fn receiver_stages_verifies_and_atomically_commits() {
    let root = tempdir().expect("root");
    let payload = b"hello from DeskFuse";
    let path = RelativePath::new("folder/file.txt").expect("path");
    let entry =
        TransferEntry::file(&path, payload.len() as u64, 0, sha256_bytes(payload)).expect("entry");
    let manifest = TransferManifest::new(vec![
        TransferEntry::directory("folder", 0).expect("directory"),
        entry.clone(),
    ])
    .expect("manifest");
    let receiver = FileReceiver::new(root.path()).expect("receiver");
    let id = TransferId::new();
    let mut staging = receiver.begin(id, &manifest).expect("staging");
    let descriptors = chunk_descriptors_for_data(payload, 4).expect("chunks");
    for descriptor in descriptors {
        let start = descriptor.offset as usize;
        let end = start + descriptor.length as usize;
        staging
            .write_chunk(&path, descriptor, &payload[start..end])
            .expect("chunk");
    }
    staging.complete_file(&entry).expect("hash");
    let report = receiver
        .commit_staging(staging, ConflictStrategy::Overwrite)
        .expect("commit");
    assert!(report.written.iter().any(|path| path.ends_with("file.txt")));
    assert_eq!(
        fs::read(root.path().join("folder/file.txt")).expect("read"),
        payload
    );
    assert!(!root.path().join(format!(".deskfuse-staging-{id}")).exists());
}

#[test]
fn receiver_hash_mismatch_does_not_commit_bad_file() {
    let root = tempdir().expect("root");
    let path = RelativePath::new("file.bin").expect("path");
    let entry = TransferEntry::file(&path, 4, 0, sha256_bytes(b"good")).expect("entry");
    let manifest = TransferManifest::new(vec![entry.clone()]).expect("manifest");
    let receiver = FileReceiver::new(root.path()).expect("receiver");
    let mut staging = receiver
        .begin(TransferId::new(), &manifest)
        .expect("staging");
    let descriptor = ChunkDescriptor::new(0, 0, 4)
        .expect("descriptor")
        .with_hash(sha256_bytes(b"bad"));
    assert!(staging.write_chunk(&path, descriptor, b"bad").is_err());
    assert!(staging.complete_file(&entry).is_err());
    receiver.abort_staging(staging).expect("abort");
    assert!(!root.path().join("file.bin").exists());
}

#[test]
fn duplicate_offer_key_reuses_the_same_acceptance_and_staging_path() {
    let root = tempdir().expect("root");
    let manifest = TransferManifest::new(vec![
        TransferEntry::file("file", 1, 0, sha256_bytes(b"x")).expect("entry"),
    ])
    .expect("manifest");
    let id = TransferId::new();
    let offer = TransferOffer::new(id, manifest.clone());
    let mut registry = OfferRegistry::default();
    assert_eq!(
        registry.accept(&offer).expect("offer"),
        OfferOutcome::Accepted(id)
    );
    assert_eq!(
        registry.accept(&offer).expect("duplicate"),
        OfferOutcome::Duplicate(id)
    );
    assert_eq!(registry.len(), 1);
    let receiver = FileReceiver::new(root.path()).expect("receiver");
    let first = receiver.begin(id, &manifest).expect("first staging");
    let first_path = first.path().to_path_buf();
    let second = receiver.begin(id, &manifest).expect("second staging");
    assert_eq!(first_path, second.path());
    receiver.abort_staging(first).expect("abort");
    receiver.abort_staging(second).expect("abort");
}

#[test]
fn conflict_strategies_skip_overwrite_and_rename() {
    let root = tempdir().expect("root");
    let target = root.path().join("file.txt");
    fs::write(&target, b"existing").expect("file");
    assert_eq!(
        deskfuse_transfer::resolve_conflict(&target, ConflictStrategy::Skip).expect("skip"),
        ConflictResolution::Skip(target.clone())
    );
    assert_eq!(
        deskfuse_transfer::resolve_conflict(&target, ConflictStrategy::Overwrite)
            .expect("overwrite"),
        ConflictResolution::Write(target.clone())
    );
    assert_eq!(
        deskfuse_transfer::resolve_conflict(&target, ConflictStrategy::Rename).expect("rename"),
        ConflictResolution::Write(root.path().join("file (1).txt"))
    );
}

#[test]
fn forged_offer_key_is_rejected_before_registry_mutation() {
    let manifest = TransferManifest::new(vec![
        TransferEntry::file("file", 1, 0, sha256_bytes(b"x")).expect("entry"),
    ])
    .expect("manifest");
    let transfer_id = TransferId::new();
    let forged_key =
        deskfuse_transfer::OfferKey::new(TransferId::from_bytes([42; 16]), manifest.digest());
    let offer = TransferOffer {
        transfer_id,
        manifest,
        offer_key: forged_key,
    };
    let mut registry = OfferRegistry::default();
    let error = registry.accept(&offer).expect_err("forged key must fail");
    assert!(matches!(
        error,
        deskfuse_transfer::TransferError::InvalidOfferKey {
            expected: _,
            actual
        } if actual == forged_key
    ));
    assert_eq!(registry.len(), 0);
}

#[test]
fn failed_multi_file_commit_rolls_back_published_files_and_overwrites() {
    let root = tempdir().expect("root");
    let blocked = root.path().join("blocked");
    fs::write(&blocked, b"do not replace").expect("blocking file");
    fs::write(root.path().join("first.txt"), b"old value").expect("old file");

    let first_path = RelativePath::new("first.txt").expect("path");
    let second_path = RelativePath::new("blocked/second.txt").expect("path");
    let first_entry = TransferEntry::file(&first_path, 3, 0, sha256_bytes(b"new")).expect("entry");
    let second_entry =
        TransferEntry::file(&second_path, 3, 0, sha256_bytes(b"new")).expect("entry");
    let manifest =
        TransferManifest::new(vec![first_entry.clone(), second_entry.clone()]).expect("manifest");
    let receiver = FileReceiver::new(root.path()).expect("receiver");
    let transfer_id = TransferId::new();
    let mut staging = receiver.begin(transfer_id, &manifest).expect("staging");
    for (path, entry) in [(&first_path, &first_entry), (&second_path, &second_entry)] {
        let descriptor = ChunkDescriptor::new(0, 0, 3)
            .expect("descriptor")
            .with_hash(sha256_bytes(b"new"));
        staging
            .write_chunk(path, descriptor, b"new")
            .expect("chunk");
        staging.complete_file(entry).expect("hash");
    }

    let error = receiver
        .commit_staging(staging, ConflictStrategy::Overwrite)
        .expect_err("blocked parent must fail");
    assert!(matches!(error, deskfuse_transfer::TransferError::Io(_)));
    assert_eq!(
        fs::read(root.path().join("first.txt")).expect("first"),
        b"old value"
    );
    assert_eq!(fs::read(&blocked).expect("blocked"), b"do not replace");
    assert!(!root.path().join("blocked/second.txt").exists());
}

#[cfg(unix)]
#[test]
fn receiver_rejects_symlink_components_for_staging_and_destination() {
    use std::os::unix::fs::symlink;

    let root = tempdir().expect("root");
    let outside = tempdir().expect("outside");
    let path = RelativePath::new("escape/file.txt").expect("path");
    let entry = TransferEntry::file(&path, 5, 0, sha256_bytes(b"hello")).expect("entry");
    let manifest = TransferManifest::new(vec![entry.clone()]).expect("manifest");
    let receiver = FileReceiver::new(root.path()).expect("receiver");

    let transfer_id = TransferId::new();
    let mut staging = receiver.begin(transfer_id, &manifest).expect("staging");
    symlink(outside.path(), staging.path().join("escape")).expect("staging symlink");
    let descriptor = ChunkDescriptor::new(0, 0, 5)
        .expect("descriptor")
        .with_hash(sha256_bytes(b"hello"));
    assert!(staging.write_chunk(&path, descriptor, b"hello").is_err());
    receiver.abort_staging(staging).expect("abort");
    assert!(!outside.path().join("file.txt").exists());

    symlink(outside.path(), root.path().join("escape")).expect("destination symlink");
    let mut staging = receiver
        .begin(TransferId::new(), &manifest)
        .expect("staging");
    staging
        .write_chunk(&path, descriptor, b"hello")
        .expect("safe staging");
    assert!(
        receiver
            .commit_staging(staging, ConflictStrategy::Overwrite)
            .is_err()
    );
    assert!(!outside.path().join("file.txt").exists());
}

#[cfg(unix)]
#[test]
fn received_directories_restore_sender_modes_without_touching_existing_ones() {
    use std::os::unix::fs::PermissionsExt;
    let source = tempdir().expect("source");
    let mode_of =
        |path: &std::path::Path| fs::metadata(path).expect("metadata").permissions().mode() & 0o777;
    for (path, mode) in [
        ("Plugin.driver", 0o755),
        ("Plugin.driver/Contents", 0o755),
        ("Plugin.driver/Contents/MacOS", 0o750),
        ("Shared", 0o755),
    ] {
        fs::create_dir(source.path().join(path)).expect("directory");
        fs::set_permissions(source.path().join(path), fs::Permissions::from_mode(mode))
            .expect("mode");
    }
    fs::write(
        source.path().join("Plugin.driver/Contents/MacOS/Plugin"),
        b"binary",
    )
    .expect("file");
    fs::write(source.path().join("Shared/file.txt"), b"shared").expect("file");
    // 只读目录也必须保留属主写权限，否则回滚和后续清理会失败。
    fs::create_dir(source.path().join("ReadOnly")).expect("directory");
    fs::set_permissions(
        source.path().join("ReadOnly"),
        fs::Permissions::from_mode(0o555),
    )
    .expect("mode");

    let manifest = TransferManifest::from_directory(source.path()).expect("manifest");
    let destination = tempdir().expect("destination");
    fs::create_dir(destination.path().join("Shared")).expect("existing");
    fs::set_permissions(
        destination.path().join("Shared"),
        fs::Permissions::from_mode(0o700),
    )
    .expect("mode");
    let receiver = FileReceiver::new(destination.path()).expect("receiver");
    let mut staging = receiver
        .begin(TransferId::new(), &manifest)
        .expect("staging");
    for entry in manifest.files() {
        let payload = fs::read(source.path().join(entry.path().as_str())).expect("payload");
        let path = RelativePath::new(entry.path().as_str()).expect("path");
        for descriptor in chunk_descriptors_for_data(&payload, 4).expect("chunks") {
            let start = descriptor.offset as usize;
            let end = start + descriptor.length as usize;
            staging
                .write_chunk(&path, descriptor, &payload[start..end])
                .expect("chunk");
        }
        staging.complete_file(entry).expect("hash");
    }
    receiver
        .commit_staging(staging, ConflictStrategy::Overwrite)
        .expect("commit");

    let root = destination.path();
    assert_eq!(mode_of(&root.join("Plugin.driver")), 0o755);
    assert_eq!(mode_of(&root.join("Plugin.driver/Contents")), 0o755);
    assert_eq!(mode_of(&root.join("Plugin.driver/Contents/MacOS")), 0o750);
    assert_eq!(mode_of(&root.join("ReadOnly")), 0o755);
    assert_eq!(
        mode_of(&root.join("Shared")),
        0o700,
        "已有目录的权限不应被改动"
    );
    assert_eq!(
        fs::read(root.join("Shared/file.txt")).expect("read"),
        b"shared"
    );
}
