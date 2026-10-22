import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("update_cask", Path(__file__).parents[1] / "update-cask.py")
update_cask = importlib.util.module_from_spec(spec)
spec.loader.exec_module(update_cask)

DIGEST = "a" * 64


class UpdateCaskTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.1.0"\n')
        self.dmg = self.root / "DeskFuse-arm64.dmg"
        self.dmg.write_bytes(b"test fixture, not a DMG")
        self.cask = self.root / "Casks" / "deskfuse.rb"

    def test_render_contract(self):
        rendered = update_cask.render("0.1.0", DIGEST)
        self.assertIn('cask "deskfuse" do', rendered)
        self.assertIn('version "0.1.0"', rendered)
        self.assertIn(f'sha256 "{DIGEST}"', rendered)
        self.assertIn("/releases/download/v#{version}/DeskFuse-arm64.dmg", rendered)
        self.assertIn('app "DeskFuse.app"', rendered)
        self.assertIn("depends_on arch: :arm64", rendered)
        self.assertIn("depends_on macos: :sonoma", rendered)
        self.assertNotIn("xattr", rendered)
        self.assertNotIn("caveats", rendered)
        self.assertTrue(rendered.endswith("end\n"))

    def test_retap_notice_points_legacy_tap_to_source_repository(self):
        rendered = update_cask.render("0.1.0", DIGEST, retap_notice=True)
        self.assertIn("caveats <<~EOS", rendered)
        self.assertIn(f"brew tap {update_cask.DISTRIBUTION['homebrew_tap']} {update_cask.HOMEPAGE}", rendered)
        self.assertTrue(rendered.endswith("  EOS\nend\n"))

    def test_render_rejects_invalid_values(self):
        for version in ("1.0", "v0.1.0", "0.1.0-rc1", "", "01.0.0"):
            with self.assertRaises(ValueError):
                update_cask.render(version, DIGEST)
        for digest in ("", "ABCDEF", "a" * 63, "g" * 64):
            with self.assertRaises(ValueError):
                update_cask.render("0.1.0", digest)

    def test_expected_cask_uses_workspace_version(self):
        content = update_cask.expected_cask(self.dmg, root=self.root)
        self.assertIn(f'sha256 "{hashlib.sha256(self.dmg.read_bytes()).hexdigest()}"', content)
        self.assertIn('version "0.1.0"', content)

    def test_explicit_release_version_is_independent_of_default_branch(self):
        for version in ("0.0.9", "0.2.1"):
            content = update_cask.expected_cask(self.dmg, version, self.root)
            self.assertIn(f'version "{version}"', content)

    def test_explicit_invalid_version_is_not_replaced_by_workspace_default(self):
        for version in ("", "v0.2.1", "0.2"):
            with self.assertRaises(ValueError):
                update_cask.expected_cask(self.dmg, version, self.root)

    def test_update_writes_then_reports_current(self):
        self.assertTrue(update_cask.update(self.dmg, self.cask, root=self.root))
        first = self.cask.read_text()
        self.assertFalse(update_cask.update(self.dmg, self.cask, root=self.root))
        self.assertEqual(self.cask.read_text(), first)

    def test_check_does_not_write_and_rejects_stale(self):
        self.cask.parent.mkdir(parents=True)
        self.cask.write_text("stale")
        with self.assertRaises(ValueError):
            update_cask.update(self.dmg, self.cask, check=True, root=self.root)
        self.assertEqual(self.cask.read_text(), "stale")

    def test_missing_dmg_is_reported(self):
        with self.assertRaises(FileNotFoundError):
            update_cask.expected_cask(self.root / "absent.dmg", root=self.root)


if __name__ == "__main__":
    unittest.main()
