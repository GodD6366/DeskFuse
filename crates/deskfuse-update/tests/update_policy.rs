use deskfuse_core::{
    Capabilities, ConnectionContext, InterfaceKind, LinkScope, TransportSessionId,
};
use deskfuse_update::*;
use ed25519_dalek::SigningKey;
use semver::Version;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use uuid::Uuid;

fn key() -> SigningKey {
    SigningKey::from_bytes(&[31; 32])
}
fn release(build: u64) -> ReleaseVersion {
    ReleaseVersion {
        version: Version::new(0, 1, 0),
        build_number: build,
        release_id: Uuid::new_v4(),
    }
}
fn manifest(build: u64) -> ReleaseManifest {
    ReleaseManifest {
        schema_version: 1,
        application_id: "com.deskfuse.app".into(),
        release: release(build),
        protocol_major: 2,
        minimum_update_protocol: 1,
        artifacts: vec![ArtifactDescriptor {
            target: Target::MacosArm64,
            kind: PackageKind::MacosAppZip,
            file_name: "DeskFuse-arm64.zip".into(),
            size: 4,
            sha256: hex::encode(Sha256::digest(b"test")),
            minimum_os: Version::new(14, 0, 0),
        }],
    }
}
fn verified(manifest: ReleaseManifest) -> VerifiedRelease {
    SignedReleaseManifest::sign(manifest, &key())
        .unwrap()
        .verify(&key().verifying_key())
        .unwrap()
}
fn installation<'a>(
    release: &'a ReleaseVersion,
    os: &'a Version,
    quarantine: &'a BTreeSet<Uuid>,
) -> Installation<'a> {
    Installation {
        release,
        target: Target::MacosArm64,
        os,
        protocol_major: 2,
        update_protocol: 1,
        quarantined: quarantine,
    }
}
fn downloaded() -> (tempfile::TempDir, UpdateTransaction) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("payload.zip");
    std::fs::write(&path, b"test").unwrap();
    let release = verified(manifest(2));
    let artifact = verify_artifact(&release, Target::MacosArm64, &path).unwrap();
    let mut tx = UpdateTransaction::new(&release, Target::MacosArm64).unwrap();
    tx.received(0, 4).unwrap();
    tx.verified(&artifact).unwrap();
    (dir, tx)
}
fn starting() -> (tempfile::TempDir, UpdateTransaction) {
    let (dir, mut tx) = downloaded();
    assert!(tx.stage(ActiveWork::default()).unwrap());
    assert!(tx.begin_install(ActiveWork::default()).unwrap());
    tx.process_started(1000).unwrap();
    (dir, tx)
}

#[test]
fn signed_manifest_roundtrip_and_pinned_key() {
    let signed = SignedReleaseManifest::sign(manifest(2), &key()).unwrap();
    let bytes = serde_json::to_vec(&signed).unwrap();
    let parsed = SignedReleaseManifest::from_slice(&bytes).unwrap();
    assert_eq!(
        parsed.verify(&key().verifying_key()).unwrap().manifest(),
        &signed.manifest
    );
    assert!(matches!(
        parsed.verify(&SigningKey::from_bytes(&[32; 32]).verifying_key()),
        Err(UpdateError::Signature)
    ));
}
#[test]
fn tampering_with_version_or_package_invalidates_signature() {
    let signed = SignedReleaseManifest::sign(manifest(2), &key()).unwrap();
    let mut tampered = signed.clone();
    tampered.manifest.release.build_number += 1;
    assert!(tampered.verify(&key().verifying_key()).is_err());
    let mut tampered = signed;
    tampered.manifest.artifacts[0].sha256 = "a".repeat(64);
    assert!(tampered.verify(&key().verifying_key()).is_err());
}
#[test]
fn malicious_metadata_rejected_before_signature() {
    for path in [
        "../../evil.exe",
        "DeskFuse-../evil.zip",
        "DeskFuse-evil\\x.zip",
        "DeskFuse-evil.zip:stream",
    ] {
        let mut m = manifest(2);
        m.artifacts[0].file_name = path.into();
        assert!(m.validate().is_err(), "{path}");
    }
    let mut m = manifest(2);
    m.artifacts[0].kind = PackageKind::WindowsNsisExe;
    assert!(m.validate().is_err());
    let mut m = manifest(2);
    m.artifacts[0].kind = PackageKind::MacosDmg;
    m.artifacts[0].file_name = "DeskFuse-arm64.dmg".into();
    assert!(m.artifacts[0].validate().is_ok());
    assert!(m.validate().is_err());
    let mut m = manifest(2);
    m.artifacts.push(m.artifacts[0].clone());
    assert!(m.validate().is_err());
    let mut m = manifest(2);
    m.artifacts[0].size = UPDATE_CACHE_LIMIT + 1;
    assert!(m.validate().is_err());
}
#[test]
fn unknown_fields_and_oversized_manifest_are_rejected() {
    assert!(SignedReleaseManifest::from_slice(&vec![b' '; MAX_MANIFEST_BYTES + 1]).is_err());
    let signed = SignedReleaseManifest::sign(manifest(2), &key()).unwrap();
    let mut value = serde_json::to_value(signed).unwrap();
    value["download_command"] = "run this".into();
    assert!(SignedReleaseManifest::from_slice(&serde_json::to_vec(&value).unwrap()).is_err());
}
#[test]
fn artifact_checks_content_length_and_target() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("payload");
    let release = verified(manifest(2));
    std::fs::write(&path, b"test").unwrap();
    assert!(verify_artifact(&release, Target::MacosArm64, &path).is_ok());
    assert!(verify_artifact(&release, Target::WindowsX64, &path).is_err());
    std::fs::write(&path, b"evil").unwrap();
    assert!(verify_artifact(&release, Target::MacosArm64, &path).is_err());
    std::fs::write(&path, b"test-extra").unwrap();
    assert!(verify_artifact(&release, Target::MacosArm64, &path).is_err());
}
#[cfg(unix)]
#[test]
fn symlink_artifact_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("payload");
    std::fs::write(&path, b"test").unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(path, &link).unwrap();
    assert!(verify_artifact(&verified(manifest(2)), Target::MacosArm64, &link).is_err());
}
#[test]
fn version_precedence_not_git_or_lexical() {
    let mut a = release(10);
    let mut b = release(2);
    assert!(a.precedence(&b).is_gt());
    a.version = Version::parse("0.9.0").unwrap();
    b.version = Version::parse("0.10.0").unwrap();
    assert!(b.precedence(&a).is_gt());
    a.version = Version::parse("1.0.0-beta.10").unwrap();
    b.version = Version::parse("1.0.0").unwrap();
    assert!(b.precedence(&a).is_gt());
}
#[test]
fn highest_compatible_target_wins_and_quarantine_is_skipped() {
    let current = release(1);
    let os = Version::new(14, 0, 0);
    let mut quarantine = BTreeSet::new();
    let mut windows = manifest(5);
    windows.artifacts[0].target = Target::WindowsX64;
    windows.artifacts[0].kind = PackageKind::WindowsNsisExe;
    windows.artifacts[0].file_name = "DeskFuse-x64.exe".into();
    let mut too_new = manifest(6);
    too_new.artifacts[0].minimum_os = Version::new(99, 0, 0);
    let mut protocol = manifest(7);
    protocol.minimum_update_protocol = 2;
    let offers = vec![
        verified(manifest(3)),
        verified(windows),
        verified(too_new),
        verified(protocol),
        verified(manifest(2)),
    ];
    let selected = select_release(&installation(&current, &os, &quarantine), &offers)
        .unwrap()
        .unwrap();
    assert_eq!(selected.0.manifest().release.build_number, 3);
    quarantine.insert(selected.0.manifest().release.release_id);
    assert_eq!(
        select_release(&installation(&current, &os, &quarantine), &offers)
            .unwrap()
            .unwrap()
            .0
            .manifest()
            .release
            .build_number,
        2
    );
}
#[test]
fn same_build_conflict_is_not_arrival_order_dependent() {
    let current = release(1);
    let os = Version::new(14, 0, 0);
    let quarantine = BTreeSet::new();
    let m = manifest(2);
    let mut conflicting = m.clone();
    conflicting.artifacts[0].sha256 = "a".repeat(64);
    let mut offers = vec![verified(m.clone()), verified(conflicting)];
    for _ in 0..2 {
        assert!(matches!(
            select_release(&installation(&current, &os, &quarantine), &offers),
            Err(UpdateError::Conflict)
        ));
        offers.reverse();
    }
    let same = vec![verified(m.clone()), verified(m)];
    assert!(
        select_release(&installation(&current, &os, &quarantine), &same)
            .unwrap()
            .is_some()
    );
}
#[test]
fn no_downgrade_or_same_build_reinstall() {
    let m = manifest(2);
    let current = m.release.clone();
    let offers = vec![verified(m), verified(manifest(1))];
    assert!(
        select_release(
            &installation(&current, &Version::new(14, 0, 0), &BTreeSet::new()),
            &offers
        )
        .unwrap()
        .is_none()
    );
}
#[test]
fn paired_private_ip_is_not_enough_for_update() {
    let mut context = ConnectionContext {
        session_id: TransportSessionId::new(),
        local_addr: "10.0.0.1:1234".parse().unwrap(),
        remote_addr: "10.0.0.2:1234".parse().unwrap(),
        interface_name: Some("en0".into()),
        interface_kind: InterfaceKind::Ethernet,
        scope: LinkScope::Lan,
    };
    assert!(permits_lan_update(
        true,
        Capabilities::LAN_UPDATES,
        &context
    ));
    assert!(!permits_lan_update(
        false,
        Capabilities::LAN_UPDATES,
        &context
    ));
    assert!(!permits_lan_update(true, Capabilities::NONE, &context));
    context.scope = LinkScope::Unknown;
    assert!(!permits_lan_update(
        true,
        Capabilities::LAN_UPDATES,
        &context
    ));
    context.scope = LinkScope::Lan;
    context.interface_kind = InterfaceKind::Vpn;
    assert!(!permits_lan_update(
        true,
        Capabilities::LAN_UPDATES,
        &context
    ));
    context.interface_kind = InterfaceKind::Ethernet;
    context.remote_addr = "127.0.0.1:1234".parse().unwrap();
    assert!(!permits_lan_update(
        true,
        Capabilities::LAN_UPDATES,
        &context
    ));
}
#[test]
fn refuses_out_of_order_duplicate_and_oversized_chunks() {
    let mut tx = UpdateTransaction::new(&verified(manifest(2)), Target::MacosArm64).unwrap();
    assert!(tx.received(1, 1).is_err());
    assert!(tx.received(0, 5).is_err());
    assert!(tx.received(0, 0).is_err());
    tx.received(0, 2).unwrap();
    assert!(tx.received(0, 2).is_err());
    tx.received(2, 2).unwrap();
    assert_eq!(tx.status().phase, UpdatePhase::Verifying);
    assert!(tx.received(4, 1).is_err());
}
#[test]
fn waits_for_each_activity_and_rechecks_after_staging() {
    let (_dir, mut tx) = downloaded();
    for work in [
        ActiveWork {
            receiving: true,
            ..Default::default()
        },
        ActiveWork {
            pasting: true,
            ..Default::default()
        },
        ActiveWork {
            exporting: true,
            ..Default::default()
        },
        ActiveWork {
            controlling: true,
            ..Default::default()
        },
    ] {
        assert!(!tx.stage(work).unwrap());
    }
    assert!(tx.stage(ActiveWork::default()).unwrap());
    assert!(
        !tx.begin_install(ActiveWork {
            pasting: true,
            ..Default::default()
        })
        .unwrap()
    );
    assert_eq!(tx.status().phase, UpdatePhase::WaitingForIdle);
}
#[test]
fn self_test_timeout_rolls_back_and_quarantines() {
    let (_dir, mut tx) = starting();
    assert!(!tx.timed_out(60_999).unwrap());
    assert!(tx.timed_out(61_000).unwrap());
    assert_eq!(tx.status().phase, UpdatePhase::RollingBack);
    tx.rollback_complete().unwrap();
    assert_eq!(tx.status().phase, UpdatePhase::Quarantined);
}
#[test]
fn late_self_test_does_not_override_timeout() {
    let (_dir, mut tx) = starting();
    let status = tx.status().clone();
    assert!(
        tx.self_test_ok(
            status.transaction_id,
            &status.release,
            &status.artifact.sha256,
            61_000
        )
        .is_err()
    );
    assert_eq!(tx.status().phase, UpdatePhase::RollingBack);
}
#[test]
fn local_success_does_not_roll_back_when_network_is_absent() {
    let (_dir, mut tx) = starting();
    let status = tx.status().clone();
    assert!(
        tx.self_test_ok(
            Uuid::new_v4(),
            &status.release,
            &status.artifact.sha256,
            2000
        )
        .is_err()
    );
    tx.self_test_ok(
        status.transaction_id,
        &status.release,
        &status.artifact.sha256,
        2000,
    )
    .unwrap();
    assert!(!tx.timed_out(9_999_999).unwrap());
    assert!(tx.fail(FailureReason::Interrupted).is_err());
    assert!(tx.confirm_reconnect(&release(2), &status.artifact).is_err());
    tx.confirm_reconnect(&status.release, &status.artifact)
        .unwrap();
    assert_eq!(tx.status().phase, UpdatePhase::Complete);
}
#[test]
fn cancel_cannot_abandon_an_install_in_progress() {
    let (_dir, mut tx) = starting();
    assert!(tx.cancel().is_err());
    let (_dir, mut tx) = downloaded();
    tx.cancel().unwrap();
    assert_eq!(tx.status().phase, UpdatePhase::Cancelled);
    assert!(tx.stage(ActiveWork::default()).is_err());
}

#[test]
fn release_ledger_is_monotonic_and_rejects_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger");
    let first = allocate_release(Version::new(0, 1, 0), &path).unwrap();
    let second = allocate_release(Version::new(0, 1, 0), &path).unwrap();
    assert_eq!(first.build_number, 1);
    assert_eq!(second.build_number, 2);
    assert_ne!(first.release_id, second.release_id);
    std::fs::write(&path, b"1\n2").unwrap();
    assert!(allocate_release(Version::new(0, 1, 0), &path).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"1\n2");
}
#[test]
fn concurrent_release_allocations_are_serialized() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ledger");
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let path = path.clone();
            std::thread::spawn(move || {
                allocate_release(Version::new(0, 1, 0), &path)
                    .unwrap()
                    .build_number
            })
        })
        .collect();
    let mut builds: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    builds.sort_unstable();
    assert_eq!(builds, (1..=8).collect::<Vec<_>>());
}

#[test]
fn cache_import_reopen_deduplicate_and_reject_tampering() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::write(&source, b"test").unwrap();
    let signed = SignedReleaseManifest::sign(manifest(3), &key()).unwrap();
    let bytes = serde_json::to_vec(&signed).unwrap();
    let root = dir.path().join("updates");
    let cache = ReleaseCache::open(&root, key().verifying_key(), UPDATE_CACHE_LIMIT).unwrap();
    let first = cache.import(&bytes, Target::MacosArm64, &source).unwrap();
    let second = cache.import(&bytes, Target::MacosArm64, &source).unwrap();
    assert_eq!(first.artifact.path(), second.artifact.path());
    assert_eq!(cache.inventory().unwrap().len(), 1);
    assert!(ReleaseCache::open(&root, key().verifying_key(), UPDATE_CACHE_LIMIT).is_err());
    drop(cache);
    let cache = ReleaseCache::open(&root, key().verifying_key(), UPDATE_CACHE_LIMIT).unwrap();
    assert_eq!(cache.inventory().unwrap().len(), 1);
    std::fs::write(first.artifact.path(), b"evil").unwrap();
    assert!(matches!(
        cache.inventory(),
        Err(UpdateError::ArtifactMismatch)
    ));
}

#[test]
fn cache_cross_platform_inventory_only_lists_present_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::write(&source, b"test").unwrap();
    let mut manifest = manifest(4);
    let mut windows = manifest.artifacts[0].clone();
    windows.target = Target::WindowsX64;
    windows.kind = PackageKind::WindowsNsisExe;
    windows.file_name = "DeskFuse-x64.exe".into();
    windows.minimum_os = Version::new(10, 0, 22000);
    manifest.artifacts.push(windows);
    let signed = SignedReleaseManifest::sign(manifest, &key()).unwrap();
    let bytes = serde_json::to_vec(&signed).unwrap();
    let cache = ReleaseCache::open(
        &dir.path().join("updates"),
        key().verifying_key(),
        UPDATE_CACHE_LIMIT,
    )
    .unwrap();
    cache.import(&bytes, Target::WindowsX64, &source).unwrap();
    assert_eq!(
        cache.inventory().unwrap()[0].artifact.descriptor().target,
        Target::WindowsX64
    );
    cache.import(&bytes, Target::MacosArm64, &source).unwrap();
    assert_eq!(cache.inventory().unwrap().len(), 2);
}

#[test]
fn cache_bad_signature_capacity_and_version_conflict_leave_original_intact() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::write(&source, b"test").unwrap();
    let signed = SignedReleaseManifest::sign(manifest(5), &key()).unwrap();
    let bytes = serde_json::to_vec(&signed).unwrap();
    let root = dir.path().join("updates");
    let cache = ReleaseCache::open(&root, key().verifying_key(), 1).unwrap();
    assert!(cache.import(&bytes, Target::MacosArm64, &source).is_err());
    assert!(cache.inventory().unwrap().is_empty());
    drop(cache);
    let cache = ReleaseCache::open(&root, key().verifying_key(), UPDATE_CACHE_LIMIT).unwrap();
    cache.import(&bytes, Target::MacosArm64, &source).unwrap();
    let conflict = SignedReleaseManifest::sign(manifest(5), &key()).unwrap();
    assert!(matches!(
        cache.import(
            &serde_json::to_vec(&conflict).unwrap(),
            Target::MacosArm64,
            &source
        ),
        Err(UpdateError::Conflict)
    ));
    let mut bad = signed;
    bad.signature = "00".repeat(64);
    assert!(matches!(
        cache.import(
            &serde_json::to_vec(&bad).unwrap(),
            Target::MacosArm64,
            &source
        ),
        Err(UpdateError::Signature)
    ));
    assert_eq!(cache.inventory().unwrap().len(), 1);
}

#[test]
fn cache_never_advertises_partial_staging_and_counts_it_against_budget() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("updates");
    let cache = ReleaseCache::open(&root, key().verifying_key(), 64).unwrap();
    std::fs::create_dir(root.join("stage-incomplete")).unwrap();
    std::fs::write(root.join("stage-incomplete/partial"), [0; 64]).unwrap();
    assert!(cache.inventory().unwrap().is_empty());
    let source = dir.path().join("source");
    std::fs::write(&source, b"test").unwrap();
    let signed = SignedReleaseManifest::sign(manifest(6), &key()).unwrap();
    assert!(
        cache
            .import(
                &serde_json::to_vec(&signed).unwrap(),
                Target::MacosArm64,
                &source
            )
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn cache_rejects_symlink_payload() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    std::fs::write(&source, b"test").unwrap();
    let signed = SignedReleaseManifest::sign(manifest(7), &key()).unwrap();
    let cache = ReleaseCache::open(
        &dir.path().join("updates"),
        key().verifying_key(),
        UPDATE_CACHE_LIMIT,
    )
    .unwrap();
    let cached = cache
        .import(
            &serde_json::to_vec(&signed).unwrap(),
            Target::MacosArm64,
            &source,
        )
        .unwrap();
    std::fs::remove_file(cached.artifact.path()).unwrap();
    std::os::unix::fs::symlink(&source, cached.artifact.path()).unwrap();
    assert!(cache.inventory().is_err());
}

#[test]
fn cache_eviction_keeps_installed_and_inflight_releases() {
    let dir = tempfile::tempdir().unwrap();
    let payload = dir.path().join("source.zip");
    std::fs::write(&payload, b"test").unwrap();
    let cache = ReleaseCache::open(&dir.path().join("cache"), key().verifying_key(), 2500).unwrap();
    let first = SignedReleaseManifest::sign(manifest(1), &key()).unwrap();
    let second = SignedReleaseManifest::sign(manifest(2), &key()).unwrap();
    cache
        .import(
            &serde_json::to_vec(&first).unwrap(),
            Target::MacosArm64,
            &payload,
        )
        .unwrap();
    cache
        .import(
            &serde_json::to_vec(&second).unwrap(),
            Target::MacosArm64,
            &payload,
        )
        .unwrap();
    let pinned = BTreeSet::from([second.manifest.release.release_id]);
    cache.reserve(1700, &pinned).unwrap();
    let remaining = cache.inventory().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].artifact.release(), &second.manifest.release);
    assert!(cache.reserve(2500, &pinned).is_err());
    assert_eq!(cache.inventory().unwrap().len(), 1);
}
