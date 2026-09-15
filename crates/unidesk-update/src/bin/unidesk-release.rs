//! Offline coordinator; never send its key file to build or distribution peers.
use ed25519_dalek::{SigningKey, VerifyingKey};
use std::{
    error::Error,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
};
use unidesk_update::{
    MAX_MANIFEST_BYTES, ReleaseCache, ReleaseManifest, SignedReleaseManifest, Target,
    UPDATE_CACHE_LIMIT, allocate_release, verify_artifact,
};

const USAGE: &str = "unidesk-release init-key KEY_FILE PUBLIC_KEY_FILE\n\
unidesk-release next VERSION LEDGER RELEASE_JSON\n\
unidesk-release sign KEY_FILE UNSIGNED_MANIFEST ARTIFACT_DIR SIGNED_MANIFEST\n\
unidesk-release verify PUBLIC_KEY_FILE SIGNED_MANIFEST ARTIFACT_DIR\n\
unidesk-release cache-import PUBLIC_KEY_FILE SIGNED_MANIFEST TARGET ARTIFACT CACHE_DIR\n\
unidesk-release cache-list PUBLIC_KEY_FILE CACHE_DIR\n\
Keep KEY_FILE and LEDGER outside the repository; artifact files are never executed.";

fn read_bounded(path: &Path, max: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    let file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(max as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err("input file is too large".into());
    }
    Ok(bytes)
}
fn create_file(path: &Path, bytes: &[u8], secret: bool) -> Result<(), Box<dyn Error>> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if secret { 0o600 } else { 0o644 });
    }
    #[cfg(not(unix))]
    if secret {
        return Err("initialize the release key on the macOS coordinator".into());
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn read_key(path: &Path, secret: bool) -> Result<[u8; 32], Box<dyn Error>> {
    let metadata = path.symlink_metadata()?;
    if !metadata.is_file() {
        return Err("key must be a regular file".into());
    }
    #[cfg(unix)]
    if secret {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("private key must not be readable by group or others".into());
        }
    }
    #[cfg(not(unix))]
    if secret {
        return Err("private signing is restricted to the macOS coordinator".into());
    }
    let raw = read_bounded(path, 128)?;
    let text = std::str::from_utf8(&raw)?;
    let mut bytes = [0u8; 32];
    hex::decode_to_slice(text.trim(), &mut bytes)?;
    Ok(bytes)
}
fn import_cache(
    key_path: &str,
    input: &str,
    target: &str,
    source: &str,
    root: &str,
) -> Result<(), Box<dyn Error>> {
    let public = VerifyingKey::from_bytes(&read_key(Path::new(key_path), false)?)?;
    let target = match target {
        "macos-arm64" => Target::MacosArm64,
        "windows-x64" => Target::WindowsX64,
        _ => return Err("target must be macos-arm64 or windows-x64".into()),
    };
    let cache = ReleaseCache::open(Path::new(root), public, UPDATE_CACHE_LIMIT)?;
    let cached = cache.import(
        &read_bounded(Path::new(input), MAX_MANIFEST_BYTES)?,
        target,
        Path::new(source),
    )?;
    println!(
        "Cached {:?} build {} SHA-256 {}",
        target,
        cached.artifact.release().build_number,
        cached.artifact.descriptor().sha256
    );
    Ok(())
}
fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [help] if help == "--help" || help == "-h" => {
            println!("{USAGE}");
        }
        [command, key_path, public_path] if command == "init-key" => {
            if Path::new(key_path).exists() || Path::new(public_path).exists() {
                return Err(
                    "key already exists; never rotate an installed trust root implicitly".into(),
                );
            }
            let key = SigningKey::generate(&mut rand_core::OsRng);
            create_file(
                Path::new(key_path),
                hex::encode(key.to_bytes()).as_bytes(),
                true,
            )?;
            create_file(
                Path::new(public_path),
                hex::encode(key.verifying_key().to_bytes()).as_bytes(),
                false,
            )?;
            println!("Created release key and public key; private material was not printed.");
        }
        [command, version, ledger, out] if command == "next" => {
            let release = allocate_release(version.parse()?, Path::new(ledger))?;
            create_file(Path::new(out), &serde_json::to_vec_pretty(&release)?, false)?;
            println!(
                "Allocated {} build {} ({})",
                release.version, release.build_number, release.release_id
            );
        }
        [command, key_path, input, dir, out] if command == "sign" => {
            let key = SigningKey::from_bytes(&read_key(Path::new(key_path), true)?);
            let manifest: ReleaseManifest =
                serde_json::from_slice(&read_bounded(Path::new(input), MAX_MANIFEST_BYTES)?)?;
            let signed = SignedReleaseManifest::sign(manifest, &key)?;
            let verified = signed.verify(&key.verifying_key())?;
            for artifact in &verified.manifest().artifacts {
                verify_artifact(
                    &verified,
                    artifact.target,
                    &Path::new(dir).join(&artifact.file_name),
                )?;
            }
            create_file(Path::new(out), &serde_json::to_vec_pretty(&signed)?, false)?;
            println!(
                "Signed verified release {} build {}",
                verified.manifest().release.version,
                verified.manifest().release.build_number
            );
        }
        [command, key_path, input, target, source, root] if command == "cache-import" => {
            import_cache(key_path, input, target, source, root)?;
        }
        [command, key_path, root] if command == "cache-list" => {
            let public = VerifyingKey::from_bytes(&read_key(Path::new(key_path), false)?)?;
            let cache = ReleaseCache::open(Path::new(root), public, UPDATE_CACHE_LIMIT)?;
            for cached in cache.inventory()? {
                println!(
                    "{:?} build {} SHA-256 {}",
                    cached.artifact.descriptor().target,
                    cached.artifact.release().build_number,
                    cached.artifact.descriptor().sha256
                );
            }
        }
        [command, key_path, input, dir] if command == "verify" => {
            let public = VerifyingKey::from_bytes(&read_key(Path::new(key_path), false)?)?;
            let signed = SignedReleaseManifest::from_slice(&read_bounded(
                Path::new(input),
                MAX_MANIFEST_BYTES,
            )?)?;
            let release = signed.verify(&public)?;
            for artifact in &release.manifest().artifacts {
                verify_artifact(
                    &release,
                    artifact.target,
                    &Path::new(dir).join(&artifact.file_name),
                )?;
            }
            println!(
                "Verified {} build {}: {} artifacts",
                release.manifest().release.version,
                release.manifest().release.build_number,
                release.manifest().artifacts.len()
            );
        }
        _ => return Err(USAGE.into()),
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
