import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import uuid

spec = importlib.util.spec_from_file_location("release", Path(__file__).parents[1] / "release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.data = {"version": "0.1.0", "build_number": 8, "release_id": str(uuid.uuid4())}
        self.path = self.root / "release.json"
        self.save()
        self.commit = "a" * 40

    def save(self):
        self.path.write_text(json.dumps(self.data))

    def test_valid_release(self):
        self.assertEqual(release.validate(self.path, "0.1.0", "v0.1.0"), self.data)

    def test_version_and_tag_mismatch(self):
        for version, tag in [("0.2.0", "v0.2.0"), ("0.1.0", "v0.2.0"), ("0.1.0", "v0.1.0-rc1")]:
            with self.assertRaises(ValueError):
                release.validate(self.path, version, tag)

    def test_missing_descriptor(self):
        with self.assertRaises(FileNotFoundError):
            release.validate(self.root / "absent.json", "0.1.0")

    def test_invalid_builds(self):
        for value in (0, -1, 10000, True, "8", 1.0):
            self.data["build_number"] = value
            self.save()
            with self.assertRaises(ValueError):
                release.validate(self.path, "0.1.0")

    def test_invalid_identity_and_extra_field(self):
        for value in ("bad", str(uuid.UUID(int=0))):
            self.data["release_id"] = value
            self.save()
            with self.assertRaises(ValueError):
                release.validate(self.path, "0.1.0")
        self.data["private_key"] = "never permitted"
        self.save()
        with self.assertRaises(ValueError):
            release.validate(self.path, "0.1.0")

    def make_inventory(self):
        (self.root / "DeskFuse-arm64.dmg").write_bytes(b"test fixture, not a DMG")
        (self.root / "DeskFuse-x86_64.dmg").write_bytes(b"intel fixture, not a DMG")
        windows = self.root / "DeskFuse-windows-x64-setup.exe"
        windows.write_bytes(b"windows installer fixture")
        (self.root / f"{windows.name}.manifest.json").write_text(json.dumps({
            **self.data,
            "target": "win-x64",
            "installer_sha256": release.checksum(windows),
            "authenticode": "Valid",
            "release_signed": True,
        }), encoding="utf-8-sig")
        with patch.object(release, "version", return_value=self.data["version"]), patch.object(release.subprocess, "check_output", return_value="rustc test"):
            release.inventory(self.root, self.commit)

    def make_macos_inventory(self):
        (self.root / "DeskFuse-arm64.dmg").write_bytes(b"test fixture, not a DMG")
        (self.root / "DeskFuse-x86_64.dmg").write_bytes(b"intel fixture, not a DMG")
        with patch.object(release, "version", return_value=self.data["version"]), patch.object(release.subprocess, "check_output", return_value="rustc test"):
            release.inventory(self.root, self.commit, include_windows=False)

    def test_macos_only_inventory_round_trip(self):
        self.make_macos_inventory()
        release.verify_inventory(self.root, self.data, self.commit, include_windows=False)
        info = json.loads((self.root / "build-info.json").read_text())
        self.assertEqual({item["target"] for item in info["targets"]}, set(release.MACOS_TARGET_ASSETS.values()))
        self.assertNotIn(release.DISTRIBUTION["assets"]["windows_x64"], (self.root / "SHA256SUMS").read_text())

    def test_inventory_round_trip_and_wrong_source(self):
        self.make_inventory()
        release.verify_inventory(self.root, self.data, self.commit)
        info = json.loads((self.root / "build-info.json").read_text())
        self.assertEqual({item["target"] for item in info["targets"]}, set(release.TARGET_ASSETS.values()))
        self.assertEqual(info["targets"][0]["signing"], "ad-hoc")
        self.assertFalse(info["targets"][0]["notarized"])
        windows = next(item for item in info["targets"] if item["target"] == "win-x64")
        self.assertEqual(windows["signing"], "authenticode")
        self.assertEqual(windows["authenticode"], "Valid")
        with self.assertRaises(ValueError):
            release.verify_inventory(self.root, self.data, "b" * 40)

    def test_inventory_records_developer_id_and_notarization(self):
        (self.root / ".signing").write_text("developer-id\ntrue\n")
        self.make_inventory()
        info = json.loads((self.root / "build-info.json").read_text())
        self.assertTrue(all(item["signing"] == "developer-id" for item in info["targets"] if item["target"].endswith("apple-darwin")))
        self.assertTrue(all(item["notarized"] for item in info["targets"] if item["target"].endswith("apple-darwin")))
        release.verify_inventory(self.root, self.data, self.commit)

    def test_notarized_ad_hoc_is_rejected(self):
        (self.root / ".signing").write_text("ad-hoc\ntrue\n")
        with self.assertRaises(ValueError):
            self.make_inventory()

    def test_invalid_signing_sidecar_is_rejected(self):
        (self.root / ".signing").write_text("unknown\nfalse\n")
        with self.assertRaises(ValueError):
            self.make_inventory()

    def test_corrupt_asset(self):
        self.make_inventory()
        (self.root / "DeskFuse-arm64.dmg").write_bytes(b"changed")
        with self.assertRaises(ValueError):
            release.verify_inventory(self.root, self.data, self.commit)

    def test_legacy_two_asset_inventory_remains_verifiable(self):
        self.data["version"] = "0.4.5"
        self.save()
        self.make_inventory()
        (self.root / "DeskFuse-windows-x64-setup.exe").unlink()
        info = json.loads((self.root / "build-info.json").read_text())
        info["targets"] = [item for item in info["targets"] if item["target"] != "win-x64"]
        (self.root / "build-info.json").write_text(json.dumps(info))
        sums = [line for line in (self.root / "SHA256SUMS").read_text().splitlines() if "windows-x64" not in line]
        sums = [f"{release.checksum(self.root / 'build-info.json')}  build-info.json" if line.endswith("  build-info.json") else line for line in sums]
        (self.root / "SHA256SUMS").write_text("\n".join(sums) + "\n")
        release.verify_inventory(self.root, self.data, self.commit)
        with self.assertRaisesRegex(ValueError, "Windows installer required"):
            release.verify_inventory(self.root, {**self.data, "version": "0.4.6"}, self.commit)

    def test_windows_manifest_must_match_release_and_signed_payload(self):
        self.make_inventory()
        path = self.root / "DeskFuse-windows-x64-setup.exe.manifest.json"
        manifest = json.loads(path.read_text(encoding="utf-8-sig"))
        for change in ({"version": "9.0.0"}, {"build_number": 99},
                       {"release_id": str(uuid.uuid4())}, {"installer_sha256": "0" * 64},
                       {"release_signed": False}, {"authenticode": "NotSigned"}):
            with self.subTest(change=change):
                path.write_text(json.dumps({**manifest, **change}), encoding="utf-8-sig")
                with patch.object(release, "version", return_value=self.data["version"]), patch.object(release.subprocess, "check_output", return_value="rustc test"):
                    with self.assertRaises(ValueError):
                        release.inventory(self.root, self.commit)

    def test_probe_allows_only_known_legacy_release_without_windows(self):
        remote = {"draft": False, "prerelease": True, "assets": [{"name": name} for name in release.ASSETS
                  if name != release.DISTRIBUTION["assets"]["windows_x64"]]}
        for candidate in ("0.4.5", "0.4.6"):
            self.data["version"] = candidate
            self.save()
            with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value=candidate), patch.object(release, "api", side_effect=[{}, remote]), patch.object(release.subprocess, "run") as download, patch.object(release, "verify_inventory") as verify:
                if candidate == "0.4.5":
                    self.assertTrue(release.probe("owner/repo", f"v{candidate}", self.commit))
                    download.assert_called_once()
                    verify.assert_called_once()
                    self.assertFalse(verify.call_args.kwargs["include_windows"])
                else:
                    with self.assertRaisesRegex(ValueError, "assets conflict"):
                        release.probe("owner/repo", f"v{candidate}", self.commit)
                    download.assert_not_called()

    def test_probe_macos_only_release_requires_explicit_mode(self):
        remote = {"draft": False, "prerelease": True,
                  "assets": [{"name": name} for name in release.MACOS_ASSETS]}
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value=self.data["version"]), patch.object(release, "api", side_effect=[{}, remote]), patch.object(release.subprocess, "run") as download, patch.object(release, "verify_inventory") as verify:
            self.assertTrue(release.probe("owner/repo", "v0.1.0", self.commit, include_windows=False))
            download.assert_called_once()
            self.assertFalse(verify.call_args.kwargs["include_windows"])
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value=self.data["version"]), patch.object(release, "api", side_effect=[{}, remote]), patch.object(release.subprocess, "run") as download:
            with self.assertRaisesRegex(ValueError, "assets conflict"):
                release.probe("owner/repo", "v0.1.0", self.commit)
            download.assert_not_called()

    def test_publish_macos_only_uploads_only_selected_assets(self):
        self.make_macos_inventory()
        (self.root / "CHANGELOG.md").write_text("## [0.1.0]\nTest release")
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value=self.data["version"]), patch.object(release, "probe", side_effect=[False, True]) as probe, patch.object(release.subprocess, "run") as runner, patch.object(release, "verify_inventory") as verify:
            release.publish(self.root, "owner/repo", "v0.1.0", self.commit, include_windows=False)
            upload = next(call.args[0] for call in runner.call_args_list if call.args[0][2] == "upload")
            self.assertEqual({Path(path).name for path in upload[6:]}, set(release.MACOS_ASSETS))
            self.assertTrue(all(call.kwargs["include_windows"] is False for call in probe.call_args_list))
            self.assertTrue(all(call.kwargs["include_windows"] is False for call in verify.call_args_list))

    def test_missing_asset(self):
        self.make_inventory()
        (self.root / "build-info.json").unlink()
        with self.assertRaises(FileNotFoundError):
            release.verify_inventory(self.root, self.data, self.commit)

    def test_manifest_path_traversal_or_duplicates(self):
        self.make_inventory()
        sums = self.root / "SHA256SUMS"
        for content in ("a" * 64 + "  ../outside\n", sums.read_text() * 2):
            sums.write_text(content)
            with self.assertRaises(ValueError):
                release.verify_inventory(self.root, self.data, self.commit)

    def test_prepare_never_overwrites(self):
        with patch.object(release.subprocess, "run") as run:
            with self.assertRaises(ValueError):
                release.prepare(self.path)
            run.assert_not_called()

    def test_missing_release_is_not_missing_repo_access(self):
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value="0.1.0"), patch.object(release, "api", side_effect=ValueError("HTTP 403")):
            with self.assertRaises(ValueError):
                release.probe("owner/repo", "v0.1.0", self.commit)

    def test_existing_draft_is_not_overwritten(self):
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value="0.1.0"), patch.object(release, "api", side_effect=[{}, {"draft": True}]):
            with self.assertRaises(ValueError):
                release.probe("owner/repo", "v0.1.0", self.commit)

    def test_failed_build_does_not_create_release(self):
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value="0.1.0"), patch.object(release.subprocess, "run") as run:
            with self.assertRaises(FileNotFoundError):
                release.publish(self.root, "owner/repo", "v0.1.0", self.commit)
            run.assert_not_called()

    def test_existing_published_release_does_not_upload(self):
        self.make_inventory()
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value="0.1.0"), patch.object(release, "probe", return_value=True), patch.object(release.subprocess, "run") as run:
            release.publish(self.root, "owner/repo", "v0.1.0", self.commit)
            run.assert_not_called()

    def test_upload_failure_leaves_draft_unpublished(self):
        self.make_inventory()
        (self.root / "CHANGELOG.md").write_text("## [0.1.0]\nTest release")
        def run(command, **kwargs):
            if command[2] == "upload":
                raise release.subprocess.CalledProcessError(1, command)
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value="0.1.0"), patch.object(release, "probe", return_value=False), patch.object(release.subprocess, "run", side_effect=run) as runner:
            with self.assertRaises(release.subprocess.CalledProcessError):
                release.publish(self.root, "owner/repo", "v0.1.0", self.commit)
            self.assertEqual([call.args[0][2] for call in runner.call_args_list], ["create", "upload"])
            self.assertIn("--draft", runner.call_args_list[0].args[0])

    def test_download_mismatch_never_publishes(self):
        self.make_inventory()
        (self.root / "CHANGELOG.md").write_text("## [0.1.0]\nTest release")
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value="0.1.0"), patch.object(release, "probe", return_value=False), patch.object(release.subprocess, "run") as runner, patch.object(release, "verify_inventory", side_effect=[None, ValueError("uploaded checksum mismatch")]):
            with self.assertRaises(ValueError):
                release.publish(self.root, "owner/repo", "v0.1.0", self.commit)
            self.assertEqual([call.args[0][2] for call in runner.call_args_list], ["create", "upload", "download"])

    def test_publish_order_and_confirmation(self):
        self.make_inventory()
        (self.root / "CHANGELOG.md").write_text("## [0.1.0]\nTest release")
        notes = []
        def run(command, **kwargs):
            if command[2] == "create":
                notes.append(Path(command[command.index("--notes-file") + 1]).read_text())
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value="0.1.0"), patch.object(release, "probe", side_effect=[False, True]) as probe, patch.object(release.subprocess, "run", side_effect=run) as runner, patch.object(release, "verify_inventory"):
            release.publish(self.root, "owner/repo", "v0.1.0", self.commit)
            self.assertEqual([call.args[0][2] for call in runner.call_args_list], ["create", "upload", "download", "edit"])
            self.assertEqual(probe.call_count, 2)
            self.assertIn("--prerelease", runner.call_args_list[-1].args[0])
            self.assertNotIn("--clobber", runner.call_args_list[1].args[0])
            self.assertIn("ad-hoc signed, not notarized", notes[0])

    def test_publish_notes_for_notarized_build(self):
        (self.root / ".signing").write_text("developer-id\ntrue\n")
        self.make_inventory()
        (self.root / "CHANGELOG.md").write_text("## [0.1.0]\nTest release")
        notes = []
        def run(command, **kwargs):
            if command[2] == "create":
                notes.append(Path(command[command.index("--notes-file") + 1]).read_text())
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value="0.1.0"), patch.object(release, "probe", side_effect=[False, True]), patch.object(release.subprocess, "run", side_effect=run), patch.object(release, "verify_inventory"):
            release.publish(self.root, "owner/repo", "v0.1.0", self.commit)
        self.assertIn("Developer ID signed and notarized", notes[0])
        self.assertNotIn("ad-hoc signed", notes[0])


if __name__ == "__main__":
    unittest.main()
