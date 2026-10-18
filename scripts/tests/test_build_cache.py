"""Unit tests for the macOS build-cache helper decisions."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


def run_bash(script: str, *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["bash", "-c", script, "build-cache-test", *args],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )


class CacheFailureClassificationTests(unittest.TestCase):
    def classify(self, log: str, scope: str) -> bool:
        with tempfile.NamedTemporaryFile("w", suffix=".log", delete=False) as handle:
            handle.write(log)
            path = handle.name
        try:
            result = run_bash(
                'source scripts/build-cache.sh; deskfuse_failure_is_cache_suspect "$1" "$2"',
                path,
                scope,
            )
        finally:
            Path(path).unlink(missing_ok=True)
        self.assertIn(result.returncode, (0, 1), result.stderr)
        return result.returncode == 0

    def test_rust_internal_compiler_error_is_cache_suspect(self):
        self.assertTrue(self.classify("error: internal compiler error: broken", "rust"))

    def test_rust_signal_is_cache_suspect(self):
        self.assertTrue(self.classify("error: could not compile `x` (signal: 6, SIGABRT)", "rust"))

    def test_rust_corrupt_rlib_is_cache_suspect(self):
        self.assertTrue(self.classify("error: failed to load ../deps/libfoo.rlib: invalid metadata", "rust"))

    def test_rust_source_error_is_not_cache_suspect(self):
        self.assertFalse(
            self.classify(
                "error[E0308]: mismatched types\n --> crates/deskfuse-core/src/lib.rs:10:5",
                "rust",
            )
        )

    def test_swift_build_database_is_cache_suspect(self):
        self.assertTrue(self.classify("xcbuild: unable to load build database", "swift"))

    def test_swift_source_error_is_not_cache_suspect(self):
        self.assertFalse(self.classify("error: cannot find 'Foo' in scope", "swift"))

    def test_empty_log_is_not_cache_suspect(self):
        self.assertFalse(self.classify("", "rust"))

    def test_unknown_scope_is_not_cache_suspect(self):
        self.assertFalse(self.classify("internal compiler error", "unknown"))


class CachePathSafetyTests(unittest.TestCase):
    def assert_under(self, child: Path, parent: Path) -> bool:
        result = run_bash(
            'source scripts/build-cache.sh; deskfuse_assert_under "$1" "$2"',
            str(child),
            str(parent),
        )
        return result.returncode == 0

    def test_child_path_is_accepted(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            self.assertTrue(self.assert_under(parent / "target" / "release", parent))

    def test_sibling_path_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "keep").mkdir()
            self.assertFalse(self.assert_under(root / "escape", root / "keep"))
            self.assertFalse(self.assert_under(root / "keep-evil" / "file", root / "keep"))


if __name__ == "__main__":
    unittest.main()
