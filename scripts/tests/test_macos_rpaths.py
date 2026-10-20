from pathlib import Path
import platform
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/macos-rpaths.py"


@unittest.skipUnless(platform.system() == "Darwin", "requires Mach-O tools")
class MacosRpathTests(unittest.TestCase):
    def test_build_paths_removed_without_changing_bundled_library_resolution(self):
        with tempfile.TemporaryDirectory(prefix="deskfuse-rpath-") as raw:
            root = Path(raw)
            library = root / "Frameworks/libfixture.dylib"
            library.parent.mkdir()
            app = root / "MacOS/app"
            app.parent.mkdir()
            (root / "library.c").write_text("int fixture(void) { return 0; }\n")
            (root / "main.c").write_text("int fixture(void); int main(void) { return fixture(); }\n")
            subprocess.run([
                "xcrun", "clang", "-dynamiclib", str(root / "library.c"),
                "-Wl,-install_name,@rpath/libfixture.dylib", "-o", str(library),
            ], check=True)
            subprocess.run([
                "xcrun", "clang", str(root / "main.c"), "-L" + str(library.parent), "-lfixture",
                "-Wl,-rpath,/tmp/DeskFuse Build/PackageFrameworks",
                "-Wl,-rpath,/Applications/Xcode.app/Contents/Developer/usr/lib",
                "-Wl,-rpath,@executable_path/../Frameworks", "-Wl,-rpath,/usr/lib/swift",
                "-o", str(app),
            ], check=True)
            rejected = subprocess.run(["python3", str(SCRIPT), str(app)], capture_output=True, text=True)
            self.assertNotEqual(rejected.returncode, 0)
            self.assertIn("non-portable LC_RPATH", rejected.stderr)
            subprocess.run(["python3", str(SCRIPT), "--fix", str(app)], check=True, capture_output=True)
            subprocess.run(["python3", str(SCRIPT), str(app)], check=True)
            # A second pass must not change the already-normalized binary.
            normalized = app.read_bytes()
            subprocess.run(["python3", str(SCRIPT), "--fix", str(app)], check=True)
            self.assertEqual(normalized, app.read_bytes())
            subprocess.run(["codesign", "--force", "--sign", "-", str(app)], check=True, capture_output=True)
            subprocess.run([str(app)], check=True)


if __name__ == "__main__":
    unittest.main()
