#!/usr/bin/env python3
"""Merge a previous installation's clipboard library without importing its identity.

Both applications must be stopped. SQLite backups include committed WAL data.
Existing destination records win; source databases are never modified.
"""
from __future__ import annotations

import argparse
from contextlib import ExitStack
import fcntl
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import uuid


def columns(db, schema, table):
    return [row[1] for row in db.execute(f'PRAGMA {schema}.table_info("{table}")')]


def migrate(source: Path, destination: Path) -> dict:
    source, destination = source.resolve(), destination.resolve()
    if source == destination:
        raise ValueError("source and destination must differ")
    for directory in (source, destination):
        if not (directory / "state.sqlite3").is_file():
            raise ValueError("both installations must have an existing state.sqlite3")
    with ExitStack() as stack:
        # The application's own instance lock prevents startup during import.
        for directory in sorted((source, destination)):
            lock = stack.enter_context((directory / "instance.lock").open("a+b"))
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as error:
                raise RuntimeError("quit both applications before importing") from error
        db = sqlite3.connect(destination / "state.sqlite3", isolation_level=None)
        stack.callback(db.close)
        marker = hashlib.sha256(str(source).encode()).hexdigest()
        if columns(db, "main", "clipboard_imports") and db.execute(
            "SELECT 1 FROM clipboard_imports WHERE source_id=?", (marker,)
        ).fetchone():
            return {"already_imported": True}
        backup_dir = destination / "history-import-backups" / uuid.uuid4().hex
        backup_dir.mkdir(parents=True, mode=0o700)
        os.chmod(backup_dir.parent, 0o700)
        for directory, name in ((source, "source.sqlite3"), (destination, "destination.sqlite3")):
            reader = sqlite3.connect((directory / "state.sqlite3").as_uri() + "?mode=ro", uri=True)
            backup = sqlite3.connect(backup_dir / name)
            try:
                reader.backup(backup)
                if backup.execute("PRAGMA quick_check").fetchall() != [("ok",)]:
                    raise ValueError("database integrity check failed")
            finally:
                backup.close()
                reader.close()
            os.chmod(backup_dir / name, 0o600)
        db.execute("ATTACH DATABASE ? AS prior", ((backup_dir / "source.sqlite3").as_uri() + "?mode=ro",))
        totals = {}
        db.execute("BEGIN IMMEDIATE")
        try:
            for table in ("cl_items", "cl_records", "cl_operations", "cl_blobs"):
                if set(columns(db, "main", table)) != set(columns(db, "prior", table)):
                    raise ValueError(f"unsupported schema for {table}; no changes applied")
            db.execute("CREATE TEMP TABLE existing_records AS SELECT key FROM cl_records")
            for table, identity in (("cl_items", "id"), ("cl_operations", "seq"),
                                    ("cl_records", None), ("cl_blobs", None)):
                names = [name for name in columns(db, "main", table) if name != identity]
                fields = ",".join('"' + name + '"' for name in names)
                condition = ""
                if table == "cl_operations":
                    condition = " WHERE json_extract(body,'$.key') NOT IN (SELECT key FROM existing_records)"
                before = db.execute(f"SELECT count(*) FROM {table}").fetchone()[0]
                db.execute(f"INSERT OR IGNORE INTO {table} ({fields}) SELECT {fields} FROM prior.{table}{condition}")
                totals[table] = db.execute(f"SELECT count(*) FROM {table}").fetchone()[0] - before
            # Advance local revisions without importing sync cursors or settings.
            db.execute("INSERT INTO cl_meta(key,value) VALUES('counter',CAST((SELECT COALESCE(MAX(revision),0) FROM cl_records) AS TEXT)) ON CONFLICT(key) DO UPDATE SET value=CAST(MAX(CAST(value AS INTEGER),CAST(excluded.value AS INTEGER)) AS TEXT)")
            if columns(db, "prior", "cl_file_resources"):
                raise ValueError("file archive references require a separate resource migration")
            db.execute("INSERT INTO cl_items_fts(cl_items_fts) VALUES('integrity-check')")
            db.execute("CREATE TABLE IF NOT EXISTS clipboard_imports(source_id TEXT PRIMARY KEY, imported_at TEXT NOT NULL)")
            db.execute("INSERT INTO clipboard_imports VALUES(?,datetime('now'))", (marker,))
            db.execute("COMMIT")
        except BaseException:
            db.execute("ROLLBACK")
            raise
        return {"imported": totals, "backup_directory": str(backup_dir)}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--destination", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(migrate(args.source, args.destination), indent=2))
