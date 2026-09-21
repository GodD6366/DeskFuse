//! Offline file import. The caller must hold both applications' instance locks.
use anyhow::{Context, Result, bail, ensure};
use bincode::Options;
use deskfuse_platform::PasteboardSnapshot;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

// Matches the versioned clipboard-library envelope; never rewrite opaque bytes.
#[derive(Serialize, Deserialize)]
struct SnapshotV1 {
    source: String,
    snapshot: PasteboardSnapshot,
}

fn hash_file(path: &Path) -> Result<Vec<u8>> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        digest.update(&buffer[..size]);
    }
    Ok(digest.finalize().to_vec())
}

fn copy_tree(source: &Path, target: &Path, files: &mut u64, bytes: &mut u64) -> Result<()> {
    let meta = fs::symlink_metadata(source)?;
    ensure!(
        !meta.file_type().is_symlink(),
        "file import does not follow symbolic links"
    );
    if target.exists() {
        ensure!(
            !fs::symlink_metadata(target)?.file_type().is_symlink(),
            "destination is a symbolic link"
        );
    }
    if meta.is_dir() {
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &target.join(entry.file_name()), files, bytes)?;
        }
    } else if meta.is_file() {
        if !target.exists() {
            let temporary = target.with_file_name(format!(".import-{}", uuid::Uuid::new_v4()));
            let copied = (|| -> Result<()> {
                fs::copy(source, &temporary)?;
                ensure!(
                    hash_file(source)? == hash_file(&temporary)?,
                    "copied file checksum mismatch"
                );
                // A hard link publishes without replacing an existing destination.
                fs::hard_link(&temporary, target)?;
                Ok(())
            })();
            let _ = fs::remove_file(&temporary);
            copied?;
        }
        ensure!(
            hash_file(source)? == hash_file(target)?,
            "existing imported file differs; no overwrite performed"
        );
        *files += 1;
        *bytes += meta.len();
    } else {
        bail!("unsupported file type in transfer cache");
    }
    Ok(())
}

fn mapped(value: &str, source: &Path, target: &Path) -> Result<Option<String>> {
    let url = url::Url::parse(value).ok().filter(|u| u.scheme() == "file");
    let path = if let Some(url) = &url {
        url.to_file_path()
            .map_err(|_| anyhow::anyhow!("invalid file URL"))?
    } else {
        PathBuf::from(value)
    };
    let Ok(relative) = path.strip_prefix(source) else {
        return Ok(None);
    };
    ensure!(
        relative
            .components()
            .all(|c| matches!(c, Component::Normal(_))),
        "invalid cached file path"
    );
    let new = target.join(relative);
    Ok(Some(if url.is_some() {
        url::Url::from_file_path(new)
            .map_err(|_| anyhow::anyhow!("invalid destination URL"))?
            .to_string()
    } else {
        new.to_string_lossy().into_owned()
    }))
}

fn rewrite(
    bytes: &[u8],
    source: &Path,
    target: &Path,
    missing: &mut u64,
) -> Result<Option<Vec<u8>>> {
    let mut value: SnapshotV1 = bincode::DefaultOptions::new()
        .with_limit(bytes.len() as u64)
        .deserialize(bytes)?;
    let mut changed = false;
    for path in &mut value.snapshot.file_urls {
        if let Some(new) = mapped(path, source, target)? {
            let local = url::Url::parse(&new)
                .ok()
                .and_then(|u| u.to_file_path().ok())
                .unwrap_or_else(|| PathBuf::from(&new));
            if !local.exists() {
                *missing += 1;
            }
            *path = new;
            changed = true;
        }
    }
    if !changed {
        return Ok(None);
    }
    for item in &mut value.snapshot.items {
        for rep in &mut item.representations {
            if rep.type_name == "public.file-url"
                && let Ok(text) = std::str::from_utf8(&rep.data)
                && let Some(new) = mapped(text, source, target)?
            {
                rep.data = new.into_bytes();
            }
        }
    }
    Ok(Some(bincode::DefaultOptions::new().serialize(&value)?))
}

fn remap_hashes(value: &mut serde_json::Value, hashes: &HashMap<String, String>) {
    match value {
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                if (key == "blob_hash" || key == "digest")
                    && value.as_str().is_some_and(|s| hashes.contains_key(s))
                {
                    *value = serde_json::Value::String(hashes[value.as_str().unwrap()].clone());
                } else {
                    remap_hashes(value, hashes);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                remap_hashes(value, hashes);
            }
        }
        _ => {}
    }
}

fn main() -> Result<()> {
    let args = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    ensure!(
        args.len() == 3,
        "usage: deskfuse-history-files SOURCE_CACHE DESTINATION_FILES DATABASE"
    );
    let (source, target, database) = (&args[0], &args[1], &args[2]);
    ensure!(
        source.is_absolute() && target.is_absolute() && database.is_file(),
        "absolute paths and existing database required"
    );
    ensure!(
        !target.starts_with(source) && !source.starts_with(target),
        "source and destination must not overlap"
    );
    for ancestor in target.ancestors() {
        if ancestor.exists() {
            ensure!(
                !fs::symlink_metadata(ancestor)?.file_type().is_symlink(),
                "destination ancestor is a symbolic link"
            );
        }
    }
    let marker = hex::encode(Sha256::digest(source.to_string_lossy().as_bytes()));
    let mut db = Connection::open(database)?;
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS file_imports(source_id TEXT PRIMARY KEY,files INTEGER,bytes INTEGER,missing INTEGER)")?;
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM file_imports WHERE source_id=?1)",
        [&marker],
        |r| r.get::<_, bool>(0),
    )? {
        println!("{{\"already_imported\":true}}");
        return Ok(());
    }
    let backup_root = database
        .parent()
        .context("missing data directory")?
        .join("history-import-backups")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&backup_root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&backup_root, fs::Permissions::from_mode(0o700))?;
    }
    db.execute(
        "VACUUM INTO ?1",
        [backup_root
            .join("before-files.sqlite3")
            .to_string_lossy()
            .as_ref()],
    )?;
    fs::create_dir_all(target)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(target, fs::Permissions::from_mode(0o700))?;
    }
    let (mut files, mut bytes, mut missing, mut records) = (0, 0, 0, 0);
    if source.exists() {
        copy_tree(source, target, &mut files, &mut bytes)?;
    }
    let tx = db.transaction()?;
    let mut hashes = HashMap::new();
    let blob_ids = tx
        .prepare("SELECT hash FROM cl_blobs")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for hash in blob_ids {
        let old: Vec<u8> =
            tx.query_row("SELECT bytes FROM cl_blobs WHERE hash=?1", [&hash], |r| {
                r.get(0)
            })?;
        if let Some(new) = rewrite(&old, source, target, &mut missing)? {
            let new_hash = hex::encode(Sha256::digest(&new));
            tx.execute(
                "INSERT OR IGNORE INTO cl_blobs(hash,bytes) VALUES(?1,?2)",
                params![new_hash, new],
            )?;
            hashes.insert(hash, new_hash);
        }
    }
    let ids = tx
        .prepare("SELECT id FROM cl_items WHERE payload IS NOT NULL")?
        .query_map([], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for id in ids {
        let old: Vec<u8> = tx.query_row("SELECT payload FROM cl_items WHERE id=?1", [id], |r| {
            r.get(0)
        })?;
        if let Some(new) = rewrite(&old, source, target, &mut missing)? {
            let old_hash = hex::encode(Sha256::digest(&old));
            let new_hash = hex::encode(Sha256::digest(&new));
            tx.execute("UPDATE cl_items SET payload=?1,blob_hash=?2,digest=CASE WHEN digest=?3 THEN ?2 ELSE digest END WHERE id=?4", params![new, new_hash, old_hash, id])?;
            hashes.insert(old_hash, new_hash);
            records += 1;
        }
    }
    for (table, id, column) in [
        ("cl_records", "key", "value"),
        ("cl_operations", "op_id", "body"),
    ] {
        let values = tx
            .prepare(&format!("SELECT {id},{column} FROM {table}"))?
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (key, text) in values {
            let mut value: serde_json::Value = serde_json::from_str(&text)?;
            let original = value.clone();
            remap_hashes(&mut value, &hashes);
            if value != original {
                tx.execute(
                    &format!("UPDATE {table} SET {column}=?1 WHERE {id}=?2"),
                    params![value.to_string(), key],
                )?;
            }
        }
    }
    tx.execute(
        "INSERT INTO file_imports VALUES(?1,?2,?3,?4)",
        params![marker, files, bytes, missing],
    )?;
    tx.commit()?;
    println!(
        "{}",
        serde_json::json!({"files":files,"bytes":bytes,"records":records,"missing_references":missing})
    );
    Ok(())
}
