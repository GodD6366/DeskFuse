from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
VERIFIER = ROOT / "scripts/verify-input-service.py"
PLIST = ROOT / "crates/deskfuse-ffi/resources/input-service-info.plist"


@unittest.skipUnless(sys.platform == "darwin", "requires macOS Mach-O tools")
class InputServiceIdentityTests(unittest.TestCase):
    def verify_helper(self, info):
        with tempfile.TemporaryDirectory(prefix="deskfuse-input-identity-") as raw:
            root = Path(raw)
            source = root / "helper.c"
            source.write_text("int main(void) { return 0; }\n")
            binary = root / "deskfuse-input-service"
            command = ["xcrun", "clang", str(source), "-o", str(binary)]
            if info is not None:
                plist = root / "Info.plist"
                plist.write_bytes(plistlib.dumps(info))
                command += ["-Xlinker", "-sectcreate", "-Xlinker", "__TEXT",
                            "-Xlinker", "__info_plist", "-Xlinker", str(plist)]
            subprocess.run(command, check=True, capture_output=True)
            return subprocess.run([sys.executable, str(VERIFIER), str(binary)],
                                  capture_output=True, text=True)

    def test_background_helper_passes(self):
        result = self.verify_helper(plistlib.loads(PLIST.read_bytes()))
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_missing_identity_is_rejected(self):
        result = self.verify_helper(None)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing its embedded background Info.plist", result.stderr)

    def test_foreground_helper_is_rejected(self):
        info = plistlib.loads(PLIST.read_bytes())
        info["LSBackgroundOnly"] = False
        result = self.verify_helper(info)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("LSBackgroundOnly=true", result.stderr)

    def test_main_app_identity_is_rejected(self):
        info = plistlib.loads(PLIST.read_bytes())
        info["CFBundleIdentifier"] = "com.deskfuse.app"
        result = self.verify_helper(info)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("own bundle identifier", result.stderr)


if __name__ == "__main__":
    unittest.main()
