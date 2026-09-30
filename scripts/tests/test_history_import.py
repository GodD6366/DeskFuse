import importlib.util
from pathlib import Path
import sqlite3
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("history_import", Path(__file__).parents[1] / "import-clipboard-history.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class HistoryImportTests(unittest.TestCase):
    def setup_database(self, directory, global_id, content):
        directory.mkdir()
        db = sqlite3.connect(directory / "state.sqlite3")
        db.executescript("""
            PRAGMA journal_mode=WAL;
            CREATE TABLE cl_items(id INTEGER PRIMARY KEY,global_id TEXT UNIQUE,payload BLOB);
            CREATE TABLE cl_records(key TEXT PRIMARY KEY,revision INTEGER,author TEXT,deleted INTEGER,value TEXT);
            CREATE TABLE cl_operations(seq INTEGER PRIMARY KEY,op_id TEXT UNIQUE,body TEXT);
            CREATE TABLE cl_blobs(hash TEXT PRIMARY KEY,bytes BLOB);
            CREATE TABLE cl_meta(key TEXT PRIMARY KEY,value TEXT);
            CREATE VIRTUAL TABLE cl_items_fts USING fts5(search_index);
        """)
        db.execute("INSERT INTO cl_items VALUES(1,?,?)", (global_id, content))
        db.execute("INSERT INTO cl_records VALUES('list:shared',1,'device',0,?)", (content.decode(),))
        db.commit()
        return db

    def test_wal_merge_preserves_destination_and_is_idempotent(self):
        with tempfile.TemporaryDirectory() as temp:
            source, target = Path(temp) / "source", Path(temp) / "target"
            old = self.setup_database(source, "old", b"source")
            new = self.setup_database(target, "new", b"destination")
            try:
                old.executescript("ALTER TABLE cl_items RENAME TO previous_items; CREATE TABLE cl_items(payload BLOB,id INTEGER PRIMARY KEY,global_id TEXT UNIQUE); INSERT INTO cl_items SELECT payload,id,global_id FROM previous_items; DROP TABLE previous_items;")
                result = MODULE.migrate(source, target)
                self.assertEqual(result["imported"]["cl_items"], 1)
                self.assertEqual(new.execute("SELECT count(*) FROM cl_items").fetchone()[0], 2)
                self.assertEqual(new.execute("SELECT value FROM cl_records").fetchone()[0], "destination")
                self.assertTrue(MODULE.migrate(source, target)["already_imported"])
                self.assertEqual(old.execute("SELECT count(*) FROM cl_items").fetchone()[0], 1)
            finally:
                old.close()
                new.close()

    def test_failure_rolls_back_inserted_history(self):
        with tempfile.TemporaryDirectory() as temp:
            source, target = Path(temp) / "source", Path(temp) / "target"
            old = self.setup_database(source, "old", b"source")
            new = self.setup_database(target, "new", b"destination")
            try:
                old.execute("CREATE TABLE cl_file_resources(id TEXT)")
                old.commit()
                with self.assertRaises(ValueError):
                    MODULE.migrate(source, target)
                self.assertEqual(new.execute("SELECT global_id FROM cl_items").fetchall(), [("new",)])
            finally:
                old.close()
                new.close()
