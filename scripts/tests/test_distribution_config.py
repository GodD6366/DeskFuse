import json
from pathlib import Path
import runpy
import tempfile
import unittest

MODULE = runpy.run_path(str(Path(__file__).parents[1] / "distribution-config.py"))


class DistributionConfigTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.directory = self.root / "config"
        self.directory.mkdir()
        self.product = {"schema_version": 1, "active_distribution": "active.json"}
        self.active = {
            "schema_version": 1,
            "source_repository": "owner/DeskFuse",
            "release_repository": "owner/deskfuse-release",
            "release_feed_url": "https://deskfuse.example/api/releases.json",
            "homebrew_tap": "owner/deskfuse",
            "app_bundle": "DeskFuse.app",
            "cask_token": "deskfuse",
            "assets": {"arm64": "DeskFuse-arm64.dmg", "x86_64": "DeskFuse-x86_64.dmg", "windows_x64": "DeskFuse-windows-x64-setup.exe"},
        }

    def load(self):
        (self.directory / "deskfuse.json").write_text(json.dumps(self.product))
        (self.directory / "active.json").write_text(json.dumps(self.active))
        return MODULE["load"](self.root)

    def test_reads_active_repository_instead_of_target(self):
        self.product["target"] = {"release_repository": "future/another-repo"}
        self.assertEqual(self.load()["release_repository"], "owner/deskfuse-release")

    def test_rejects_outside_configuration(self):
        self.product["active_distribution"] = "../outside.json"
        with self.assertRaises(ValueError):
            self.load()

    def test_rejects_unknown_schema(self):
        self.active["schema_version"] = 2
        with self.assertRaises(ValueError):
            self.load()

    def test_rejects_repository_url_and_workflow_injection(self):
        for value in ("https://example.com/repo", "owner/repo\noutput=bad", "owner/repo/extra"):
            self.active["release_repository"] = value
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.load()

    def test_rejects_release_feed_url_injection(self):
        for value in ("http://deskfuse.example/api/releases.json", "https://example.com\noutput=bad"):
            self.active["release_feed_url"] = value
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.load()

    def test_rejects_nested_artifact_paths(self):
        self.active["assets"]["arm64"] = "../DeskFuse.dmg"
        with self.assertRaises(ValueError):
            self.load()

    def test_rejects_duplicate_assets(self):
        self.active["assets"]["x86_64"] = self.active["assets"]["arm64"]
        with self.assertRaises(ValueError):
            self.load()

    def test_rejects_non_dmg_assets(self):
        self.active["assets"]["arm64"] = "DeskFuse-arm64.zip"
        with self.assertRaises(ValueError):
                self.load()

    def test_rejects_non_exe_windows_asset(self):
        self.active["assets"]["windows_x64"] = "DeskFuse-windows-x64.zip"
        with self.assertRaises(ValueError):
            self.load()

    def test_website_fallback_uses_release_descriptor_version(self):
        (self.root / "release.json").write_text(json.dumps({"version": "0.4.5"}))

        projection = MODULE["website_projection"](self.active, self.root)

        self.assertEqual(projection["fallback_release_tag"], "v0.4.5")

    def test_website_fallback_rejects_invalid_release_version(self):
        for version in (None, "v0.4.5", "0.4", "0.4.5\noutput=bad"):
            (self.root / "release.json").write_text(json.dumps({"version": version}))
            with self.subTest(version=version), self.assertRaises(ValueError):
                MODULE["website_projection"](self.active, self.root)


if __name__ == "__main__":
    unittest.main()
