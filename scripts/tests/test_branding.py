from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest

MODULE = runpy.run_path(str(Path(__file__).parents[1] / "check-branding.py"))
WORD = MODULE["RETIRED_KEYWORD"].upper()


class BrandingTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        subprocess.run(["git", "init", "--quiet", str(self.root)], check=True)

    def findings(self):
        return list(MODULE["violations"](self.root))

    def test_accepts_clean_source(self):
        (self.root / "DeskFuse.txt").write_text("DeskFuse")
        self.assertEqual(self.findings(), [])

    def test_checks_untracked_filenames_and_binary_contents(self):
        (self.root / f"{WORD}.bin").write_bytes(b"\0" + WORD.encode())
        self.assertEqual(len(self.findings()), 2)

    def test_checks_broken_symlink_without_following_it(self):
        (self.root / "link").symlink_to(f"missing/{WORD}")
        self.assertEqual(len(self.findings()), 1)

    def test_legacy_exemption_is_exact_not_directory_wide(self):
        directory = self.root / "config/legacy"
        directory.mkdir(parents=True)
        (directory / "distribution.json").write_text(WORD)
        (directory / "unexpected.json").write_text(WORD)
        findings = self.findings()
        self.assertEqual(len(findings), 1)
        self.assertIn("unexpected.json", findings[0])


if __name__ == "__main__":
    unittest.main()
