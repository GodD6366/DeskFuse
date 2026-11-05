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
        self.upload_url = "https://uploads.github.test/repos/owner/repo/releases/1/assets{?name,label}"

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
        (self.root / release.ARM64_APP_ZIP).write_bytes(b"test fixture, not a zip")
        (self.root / release.X86_64_APP_ZIP).write_bytes(b"intel fixture, not a zip")
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
        (self.root / release.ARM64_APP_ZIP).write_bytes(b"test fixture, not a zip")
        (self.root / release.X86_64_APP_ZIP).write_bytes(b"intel fixture, not a zip")
        with patch.object(release, "version", return_value=self.data["version"]), patch.object(release.subprocess, "check_output", return_value="rustc test"):
            release.inventory(self.root, self.commit, include_windows=False)

    def local_digests(self, names):
        return {name: release.checksum(self.root / name) for name in names}

    def test_macos_only_inventory_round_trip(self):
        self.make_macos_inventory()
        release.verify_inventory(self.root, self.data, self.commit, include_windows=False)
        info = json.loads((self.root / "build-info.json").read_text())
        self.assertEqual({item["target"] for item in info["targets"]}, set(release.MACOS_TARGET_ASSETS.values()))
        self.assertNotIn(release.DISTRIBUTION["assets"]["windows_x64"], (self.root / "SHA256SUMS").read_text())

    def test_inventory_records_supplied_rustc_version(self):
        self.make_macos_inventory()
        with patch.object(release, "version", return_value=self.data["version"]):
            release.inventory(self.root, self.commit, include_windows=False,
                              rustc_version="rustc 1.97.1 (release 2026-01-01)")
            info = json.loads((self.root / "build-info.json").read_text())
            self.assertEqual(info["rustc"], "rustc 1.97.1 (release 2026-01-01)")
            with self.assertRaises(ValueError):
                release.inventory(self.root, self.commit, include_windows=False, rustc_version="1.97.1")

    def test_verify_ci_requires_a_green_macos_check(self):
        with patch.object(release, "api", return_value={"check_runs": [
                {"name": "macOS arm64 checks", "conclusion": "success"},
                {"name": "checks / macOS arm64 checks", "conclusion": "failure"}]}):
            release.verify_ci("owner/repo", self.commit)
        for checks, message in (
                ([{"name": "macOS arm64 checks", "conclusion": "failure"}], "did not succeed"),
                ([{"name": "macOS arm64 checks", "conclusion": None}], "did not succeed"),
                ([{"name": "Windows adapters, service, and catalog checks", "conclusion": "failure"}],
                 "no `macOS arm64 checks` check run")):
            with self.subTest(message=message):
                with patch.object(release, "api", return_value={"check_runs": checks}):
                    with self.assertRaisesRegex(ValueError, message):
                        release.verify_ci("owner/repo", self.commit)

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
            with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value=candidate), patch.object(release, "api", side_effect=[{}, remote]), patch.object(release, "download_assets") as download, patch.object(release, "verify_inventory") as verify:
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
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value=self.data["version"]), patch.object(release, "api", side_effect=[{}, remote]), patch.object(release, "download_assets") as download, patch.object(release, "verify_inventory") as verify:
            self.assertTrue(release.probe("owner/repo", "v0.1.0", self.commit, include_windows=False))
            download.assert_called_once()
            self.assertFalse(verify.call_args.kwargs["include_windows"])
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value=self.data["version"]), patch.object(release, "api", side_effect=[{}, remote]), patch.object(release, "download_assets") as download:
            with self.assertRaisesRegex(ValueError, "assets conflict"):
                release.probe("owner/repo", "v0.1.0", self.commit)
            download.assert_not_called()

    def test_publish_macos_only_uploads_only_selected_assets(self):
        self.make_macos_inventory()
        (self.root / "CHANGELOG.md").write_text("## [0.1.0]\nTest release")
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value=self.data["version"]), patch.object(release, "probe", side_effect=[False, True]) as probe, patch.object(release.subprocess, "run") as runner, patch.object(release, "verify_inventory") as verify, patch.object(release, "remote_asset_digests", return_value=self.local_digests(release.MACOS_ASSETS)), patch.object(release, "remote_release", return_value={"upload_url": self.upload_url}), patch.object(release, "upload_asset") as upload:
            release.publish(self.root, "owner/repo", "v0.1.0", self.commit, include_windows=False)
            commands = [call.args[0] for call in runner.call_args_list]
            uploads = [Path(call.args[1]).name for call in upload.call_args_list]
            self.assertEqual(set(uploads), set(release.MACOS_ASSETS))
            # One upload per asset on its own connection, and no download when
            # GitHub reports the stored digests.
            self.assertEqual(len(uploads), len(release.MACOS_ASSETS))
            self.assertNotIn("download", [command[2] for command in commands])
            self.assertTrue(all(call.kwargs["include_windows"] is False for call in probe.call_args_list))
            verify.assert_called_once()

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
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value="0.1.0"), patch.object(release, "probe", return_value=False), patch.object(release.subprocess, "run") as runner, patch.object(release, "remote_release", return_value={"upload_url": self.upload_url}), patch.object(release, "upload_asset", side_effect=ValueError("upload failed")):
            with self.assertRaises(ValueError):
                release.publish(self.root, "owner/repo", "v0.1.0", self.commit)
            commands = [call.args[0][2] for call in runner.call_args_list]
            self.assertEqual(commands[0], "create")
            self.assertNotIn("edit", commands)
            self.assertIn("--draft", runner.call_args_list[0].args[0])

    def test_download_mismatch_never_publishes(self):
        self.make_inventory()
        (self.root / "CHANGELOG.md").write_text("## [0.1.0]\nTest release")
        # GitHub deployments without asset digests still verify by downloading.
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value="0.1.0"), patch.object(release, "probe", return_value=False), patch.object(release.subprocess, "run") as runner, patch.object(release, "verify_inventory", side_effect=[None, ValueError("uploaded checksum mismatch")]), patch.object(release, "remote_asset_digests", return_value=None), patch.object(release.time, "sleep"), patch.object(release, "remote_release", return_value={"upload_url": self.upload_url}), patch.object(release, "upload_asset"), patch.object(release, "download_assets") as download:
            with self.assertRaises(ValueError):
                release.publish(self.root, "owner/repo", "v0.1.0", self.commit)
            commands = [call.args[0][2] for call in runner.call_args_list]
            self.assertEqual(commands[0], "create")
            self.assertNotIn("edit", commands)
            download.assert_called_once()

    def test_digest_mismatch_never_publishes(self):
        self.make_inventory()
        (self.root / "CHANGELOG.md").write_text("## [0.1.0]\nTest release")
        digests = self.local_digests(release.ASSETS)
        digests[release.ARM64_APP_ZIP] = "0" * 64
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value="0.1.0"), patch.object(release, "probe", return_value=False), patch.object(release.subprocess, "run") as runner, patch.object(release, "verify_inventory"), patch.object(release, "remote_asset_digests", return_value=digests), patch.object(release, "remote_release", return_value={"upload_url": self.upload_url}), patch.object(release, "upload_asset"):
            with self.assertRaisesRegex(ValueError, "digest mismatch"):
                release.publish(self.root, "owner/repo", "v0.1.0", self.commit)
            self.assertNotIn("edit", [call.args[0][2] for call in runner.call_args_list])

    def test_publish_order_and_confirmation(self):
        self.make_inventory()
        (self.root / "CHANGELOG.md").write_text("## [0.1.0]\nTest release")
        notes = []
        def run(command, **kwargs):
            if command[2] == "create":
                notes.append(Path(command[command.index("--notes-file") + 1]).read_text())
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value="0.1.0"), patch.object(release, "probe", side_effect=[False, True]) as probe, patch.object(release.subprocess, "run", side_effect=run) as runner, patch.object(release, "verify_inventory"), patch.object(release, "remote_asset_digests", return_value=self.local_digests(release.ASSETS)), patch.object(release, "remote_release", return_value={"upload_url": self.upload_url}), patch.object(release, "upload_asset") as upload:
            release.publish(self.root, "owner/repo", "v0.1.0", self.commit)
            commands = [call.args[0] for call in runner.call_args_list]
            self.assertEqual(commands[0][2], "create")
            self.assertEqual(commands[-1][2], "edit")
            self.assertNotIn("download", [command[2] for command in commands])
            self.assertEqual(probe.call_count, 2)
            self.assertIn("--prerelease", commands[-1])
            self.assertEqual(len(upload.call_args_list), len(release.ASSETS))
            self.assertIn("ad-hoc signed, not notarized", notes[0])

    def test_publish_notes_for_notarized_build(self):
        (self.root / ".signing").write_text("developer-id\ntrue\n")
        self.make_inventory()
        (self.root / "CHANGELOG.md").write_text("## [0.1.0]\nTest release")
        notes = []
        def run(command, **kwargs):
            if command[2] == "create":
                notes.append(Path(command[command.index("--notes-file") + 1]).read_text())
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value="0.1.0"), patch.object(release, "probe", side_effect=[False, True]), patch.object(release.subprocess, "run", side_effect=run), patch.object(release, "verify_inventory"), patch.object(release, "remote_asset_digests", return_value=self.local_digests(release.ASSETS)), patch.object(release, "remote_release", return_value={"upload_url": self.upload_url}), patch.object(release, "upload_asset"):
            release.publish(self.root, "owner/repo", "v0.1.0", self.commit)
        self.assertIn("Developer ID signed and notarized", notes[0])
        self.assertNotIn("ad-hoc signed", notes[0])

    def test_probe_verifies_digests_without_downloading_assets(self):
        remote = {"draft": False, "prerelease": True, "assets": [
            {"name": name, "digest": "sha256:" + "a" * 64} for name in release.MACOS_ASSETS]}
        with patch.object(release, "ROOT", self.root), patch.object(release, "version", return_value=self.data["version"]), patch.object(release, "api", side_effect=[{}, remote]), patch.object(release, "download_assets") as download, patch.object(release, "verify_inventory") as verify:
            self.assertTrue(release.probe("owner/repo", "v0.1.0", self.commit, include_windows=False))
            self.assertEqual(download.call_args.args[3], {"release.json", "build-info.json", "SHA256SUMS"})
            self.assertEqual(verify.call_args.kwargs["digests"],
                             {name: "a" * 64 for name in release.MACOS_ASSETS})


if __name__ == "__main__":
    unittest.main()
