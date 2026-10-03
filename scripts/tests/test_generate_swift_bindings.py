#!/usr/bin/env python3
"""Exercise cache invalidation and failure recovery without compiling Rust."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "bindings", Path(__file__).parents[1] / "generate-swift-bindings.py"
)
bindings = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bindings)


class BindingCacheTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.library = self.root / "library"
        self.tool = self.root / "bindgen"
        self.config = self.root / "uniffi.toml"
        for path in (self.library, self.tool, self.config):
            path.write_text("initial")
        self.package = self.root / "app"
        self.mock = patch.object(bindings.subprocess, "run", side_effect=self.emit).start()
        self.addCleanup(patch.stopall)

    def emit(self, command, **kwargs):
        for name in bindings.OUTPUTS:
            (Path(command[-1]) / name).write_text(name)

    def generate(self, force=False):
        return bindings.generate(self.library, self.tool, self.package, self.config, force)

    def test_noop_keeps_output_mtime(self):
        self.assertTrue(self.generate())
        target = self.package / next(iter(bindings.OUTPUTS.values()))
        timestamp = target.stat().st_mtime_ns
        self.assertFalse(self.generate())
        self.assertEqual(self.mock.call_count, 1)
        self.assertEqual(target.stat().st_mtime_ns, timestamp)

    def test_each_input_invalidates_but_identical_output_is_not_rewritten(self):
        self.generate()
        target = self.package / next(iter(bindings.OUTPUTS.values()))
        timestamp = target.stat().st_mtime_ns
        for path in (self.library, self.tool, self.config):
            path.write_text("changed")
            self.assertTrue(self.generate())
            self.assertFalse(self.generate())
        self.assertEqual(target.stat().st_mtime_ns, timestamp)

    def test_missing_and_corrupted_output_are_repaired(self):
        self.generate()
        for name in bindings.OUTPUTS:
            path = self.package / "Generated" / name
            path.unlink()
            self.assertTrue(self.generate())
        for name, relative in bindings.OUTPUTS.items():
            path = self.package / relative
            path.write_text("corrupt")
            self.assertTrue(self.generate())
            self.assertEqual(path.read_text(), name)

    def test_failed_generator_preserves_receipt_and_outputs(self):
        self.generate()
        receipt = self.package / "SDK/.swift-bindings-fingerprint.json"
        previous = receipt.read_bytes()
        self.library.write_text("changed")
        self.mock.side_effect = subprocess.CalledProcessError(1, "bindgen")
        with self.assertRaises(subprocess.CalledProcessError):
            self.generate()
        self.assertEqual(receipt.read_bytes(), previous)
        for name, relative in bindings.OUTPUTS.items():
            self.assertEqual((self.package / relative).read_text(), name)
        self.mock.side_effect = self.emit
        self.assertTrue(self.generate())

    def test_force_and_invalid_receipt(self):
        self.generate()
        self.assertTrue(self.generate(force=True))
        (self.package / "SDK/.swift-bindings-fingerprint.json").write_text("broken")
        self.assertTrue(self.generate())


if __name__ == "__main__":
    unittest.main()
