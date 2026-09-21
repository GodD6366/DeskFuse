from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]


HELPER = r"""
set -euo pipefail
codesign() { printf '%s\n' "$*"; }
SIGNING_KIND=ad-hoc
CODESIGN_IDENTITY=-
source "$1"
deskfuse_codesign /tmp/helper
deskfuse_codesign --identifier com.example.helper /tmp/input
SIGNING_KIND=developer-id
CODESIGN_IDENTITY=ABC
deskfuse_codesign /tmp/app
"""


class MacosCodesignTests(unittest.TestCase):
    def test_empty_extra_flags_do_not_trip_nounset(self):
        result = subprocess.run(
            ["bash", "-c", HELPER, "macos-codesign-test", str(ROOT / "scripts/macos-codesign.sh")],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertEqual(
            result.stdout.splitlines(),
            [
                "--force --strict --timestamp=none --sign - /tmp/helper",
                "--force --strict --timestamp=none --identifier com.example.helper --sign - /tmp/input",
                "--force --strict --timestamp --options runtime --sign ABC /tmp/app",
            ],
        )


if __name__ == "__main__":
    unittest.main()
