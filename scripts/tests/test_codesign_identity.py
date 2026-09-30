from pathlib import Path
import os
import stat
import subprocess
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "scripts/codesign-identity.sh"
GMAIL = "DB0C94ABA14A2F6EE00A3FB80CD7456D94D2C650"
OUTLOOK = "0C96987DD20DEF490AF424594741BB48F36DF62C"


def identities(*rows: str) -> str:
    lines = [f"  {index}) {row}" for index, row in enumerate(rows, start=1)]
    lines.append(f"     {len(rows)} valid identities found")
    return "\n".join(lines) + "\n"


class CodesignIdentityTests(unittest.TestCase):
    def run_selector(self, layout, identities_text, env=None, git_common=None):
        worktree = layout / "worktree"
        script_dir = worktree / "scripts"
        script_dir.mkdir(parents=True)
        script_dir.joinpath("codesign-identity.sh").write_text(SOURCE.read_text(), encoding="utf-8")
        common = git_common if git_common is not None else layout / "main" / ".git"
        harness = textwrap.dedent(
            """\
            set -euo pipefail
            security() { printf '%s' "$FAKE_IDENTITIES"; }
            git() { printf '%s\\n' "$FAKE_GIT_COMMON"; }
            source "$1"
            printf 'IDENTITY=%s\\n' "$CODESIGN_IDENTITY"
            """
        )
        child_env = os.environ.copy()
        child_env.pop("CODESIGN_IDENTITY", None)
        if env:
            child_env.update(env)
        child_env["FAKE_IDENTITIES"] = identities_text
        child_env["FAKE_GIT_COMMON"] = str(common)
        return subprocess.run(
            ["bash", "-c", harness, "codesign-identity-test", str(script_dir / "codesign-identity.sh")],
            cwd=worktree,
            env=child_env,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_worktree_reuses_primary_checkout_pin(self):
        with tempfile.TemporaryDirectory() as raw:
            layout = Path(raw)
            main = layout / "main"
            main.mkdir()
            main.joinpath(".codesign-identity").write_text(GMAIL + "\n", encoding="utf-8")
            result = self.run_selector(
                layout,
                identities(
                    f'{OUTLOOK} "Apple Development: other@example.com (OTHER)"',
                    f'{GMAIL} "Apple Development: maintainer@example.com (TEAM)"',
                ),
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(f"IDENTITY={GMAIL}", result.stdout)
            pin = layout / "worktree" / ".codesign-identity"
            self.assertEqual(pin.read_text(encoding="utf-8"), GMAIL + "\n")
            self.assertEqual(stat.S_IMODE(pin.stat().st_mode), 0o600)

    def test_two_certificates_without_any_pin_fail(self):
        with tempfile.TemporaryDirectory() as raw:
            layout = Path(raw)
            result = self.run_selector(
                layout,
                identities(
                    f'{GMAIL} "Apple Development: maintainer@example.com (TEAM)"',
                    f'{OUTLOOK} "Apple Development: other@example.com (OTHER)"',
                ),
            )
            self.assertEqual(result.returncode, 1)
            self.assertIn("found 2", result.stderr)
            self.assertFalse((layout / "worktree" / ".codesign-identity").exists())

    def test_existing_pin_keeps_explicit_override_local(self):
        with tempfile.TemporaryDirectory() as raw:
            layout = Path(raw)
            result_root = layout / "worktree"
            result_root.mkdir()
            pin = result_root / ".codesign-identity"
            pin.write_text(GMAIL + "\n", encoding="utf-8")
            result = self.run_selector(
                layout,
                identities(
                    f'{GMAIL} "Apple Development: maintainer@example.com (TEAM)"',
                    f'{OUTLOOK} "Apple Development: other@example.com (OTHER)"',
                ),
                env={"CODESIGN_IDENTITY": OUTLOOK},
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(f"IDENTITY={OUTLOOK}", result.stdout)
            self.assertEqual(pin.read_text(encoding="utf-8"), GMAIL + "\n")

    def test_ad_hoc_request_does_not_create_pin(self):
        with tempfile.TemporaryDirectory() as raw:
            layout = Path(raw)
            result = self.run_selector(
                layout,
                identities(f'{GMAIL} "Apple Development: maintainer@example.com (TEAM)"'),
                env={"CODESIGN_IDENTITY": "-"},
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("IDENTITY=-", result.stdout)
            self.assertFalse((layout / "worktree" / ".codesign-identity").exists())


if __name__ == "__main__":
    unittest.main()
