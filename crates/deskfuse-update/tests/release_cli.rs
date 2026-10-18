#![cfg(unix)]
use deskfuse_update::{ArtifactDescriptor, PackageKind, ReleaseManifest, ReleaseVersion, Target};
use ed25519_dalek::SigningKey;
use semver::Version;
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn tool(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_deskfuse-release"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn coordinator_builds_and_verifies_a_release_without_exposing_secret() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let output = tool(root, &["init-key", "private.key", "public.key"]);
    assert!(output.status.success());
    assert_eq!(
        std::fs::metadata(root.join("private.key"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let private = std::fs::read_to_string(root.join("private.key")).unwrap();
    assert!(!String::from_utf8_lossy(&output.stdout).contains(&private));
    assert!(
        !tool(root, &["init-key", "private.key", "public.key"])
            .status
            .success()
    );
    assert!(
        tool(root, &["next", "0.1.0", "ledger", "release.json"])
            .status
            .success()
    );
    let release: ReleaseVersion =
        serde_json::from_slice(&std::fs::read(root.join("release.json")).unwrap()).unwrap();
    let manifest = ReleaseManifest {
        schema_version: 1,
        application_id: "com.deskfuse.app".into(),
        release,
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
    };
    std::fs::write(
        root.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(root.join("DeskFuse-arm64.zip"), b"test").unwrap();
    assert!(
        tool(
            root,
            &["sign", "private.key", "manifest.json", ".", "signed.json"]
        )
        .status
        .success()
    );
    assert!(
        tool(root, &["verify", "public.key", "signed.json", "."])
            .status
            .success()
    );
    assert!(
        tool(
            root,
            &[
                "cache-import",
                "public.key",
                "signed.json",
                "macos-arm64",
                "DeskFuse-arm64.zip",
                "cache"
            ]
        )
        .status
        .success()
    );
    let inventory = tool(root, &["cache-list", "public.key", "cache"]);
    assert!(inventory.status.success());
    assert!(String::from_utf8_lossy(&inventory.stdout).contains("MacosArm64 build 1"));
    std::fs::write(root.join("DeskFuse-arm64.zip"), b"evil").unwrap();
    assert!(
        !tool(root, &["verify", "public.key", "signed.json", "."])
            .status
            .success()
    );
    assert!(
        !tool(
            root,
            &["sign", "private.key", "manifest.json", ".", "bad.json"]
        )
        .status
        .success()
    );
    assert!(!root.join("bad.json").exists());
}
#[test]
fn private_key_with_open_permissions_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("private.key"),
        hex::encode(SigningKey::from_bytes(&[7; 32]).to_bytes()),
    )
    .unwrap();
    std::fs::set_permissions(
        root.join("private.key"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let output = tool(
        root,
        &["sign", "private.key", "missing.json", ".", "out.json"],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("group or others"));
}
