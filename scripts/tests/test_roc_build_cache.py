"""A cache hit must never hide changed source or missing/corrupted outputs."""
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("roc_cache", ROOT / "scripts/roc-build-cache.py")
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)


class RocCacheTests(unittest.TestCase):
    def test_source_and_sdk_changes_invalidate_even_with_preserved_mtime(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "apps/deskfuse-macos/MicrophoneNative/microphone.cpp"
            source.parent.mkdir(parents=True)
            source.write_text("before")
            settings = root / "SDKSettings.json"
            settings.write_text("sdk1")
            with patch.dict(os.environ, {"SDKROOT": str(root), "CXX": "clang++"}), patch.object(
                cache.subprocess, "check_output", return_value=b"version1"
            ):
                first = cache.input_key(root, "arm64")
                self.assertEqual(first, cache.input_key(root, "arm64"))
                stamp = source.stat()
                source.write_text("after!")
                os.utime(source, ns=(stamp.st_atime_ns, stamp.st_mtime_ns))
                self.assertNotEqual(first, cache.input_key(root, "arm64"))
                source.write_text("before")
                self.assertEqual(first, cache.input_key(root, "arm64"))
                settings.write_text("sdk2")
                self.assertNotEqual(first, cache.input_key(root, "arm64"))

    def test_missing_corrupted_and_nonexecutable_outputs_invalidate(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "target/roc-microphone/arm64"
            output.mkdir(parents=True)
            binary = output / "DeskFuse-roc"
            binary.write_bytes(b"binary")
            binary.chmod(0o755)

            def run(action, key="sources"):
                with patch.object(cache.sys, "argv", ["cache", action, str(root), "arm64", key]):
                    return cache.main()

            self.assertEqual(run("check"), 1)
            self.assertEqual(run("record"), 0)
            self.assertEqual(run("check"), 0)
            self.assertEqual(run("check", "changed"), 1)
            binary.write_bytes(b"damage")
            self.assertEqual(run("check"), 1)
            binary.write_bytes(b"binary")
            binary.chmod(0o644)
            self.assertEqual(run("check"), 1)
            binary.unlink()
            self.assertEqual(run("check"), 1)


if __name__ == "__main__":
    unittest.main()
