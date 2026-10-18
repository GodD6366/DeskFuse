from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
HELPER = ROOT / "scripts/macos-app-lifecycle.sh"


class MacosAppLifecycleTests(unittest.TestCase):
    def run_shell(self, body):
        with tempfile.TemporaryDirectory() as directory:
            calls = Path(directory) / "calls"
            result = subprocess.run(
                ["/bin/bash", "-c", 'set -euo pipefail\nsource "$1"\n' + body,
                 "lifecycle-test", str(HELPER), str(calls)],
                capture_output=True, text=True,
            )
            recorded = calls.read_text().splitlines() if calls.exists() else []
            return result, recorded

    def test_proc_not_found_retries_existing_new_instance(self):
        result, calls = self.run_shell(r'''
calls_path="$2"
open() {
  printf '%s\n' "$*" >> "$calls_path"
  if [[ "$1" == -n ]]; then
    echo '_LSOpenURLsWithCompletionHandler() failed with error -600.' >&2
    return 1
  fi
}
sleep() { :; }
deskfuse_open_installed_app '/Applications/DeskFuse.app'
''')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, ["-n /Applications/DeskFuse.app", "/Applications/DeskFuse.app"])

    def test_other_launch_error_is_not_retried(self):
        result, calls = self.run_shell(r'''
calls_path="$2"
open() { echo "$*" >> "$calls_path"; echo 'launch failed: error -10810' >&2; return 7; }
sleep() { echo unexpected-sleep >> "$calls_path"; }
deskfuse_open_installed_app '/Applications/DeskFuse.app'
''')
        self.assertEqual(result.returncode, 7)
        self.assertEqual(calls, ["-n /Applications/DeskFuse.app"])
        self.assertIn("-10810", result.stderr)

    def test_registration_retry_is_bounded(self):
        result, calls = self.run_shell(r'''
calls_path="$2"
open() { echo "$*" >> "$calls_path"; echo 'launch failed with error -600.' >&2; return 1; }
sleep() { :; }
deskfuse_open_installed_app '/Applications/DeskFuse.app'
''')
        self.assertEqual(result.returncode, 1)
        self.assertEqual(len(calls), 3)

    def test_helper_must_exit_before_replacement(self):
        result, calls = self.run_shell(r'''
calls_path="$2"
pkill() { :; }
pgrep() { [[ "$2" == deskfuse-input-service ]]; }
sleep() { echo waiting >> "$calls_path"; }
deskfuse_stop_for_install deskfuse-app
echo replaced >> "$calls_path"
''')
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(calls, ["waiting"] * 20)
        self.assertIn("keeping the installed app unchanged", result.stderr)

    def test_stop_returns_when_both_processes_exit(self):
        result, calls = self.run_shell(r'''
calls_path="$2"
pkill() { echo "stop $*" >> "$calls_path"; }
pgrep() { return 1; }
sleep() { echo unexpected-sleep >> "$calls_path"; }
deskfuse_stop_for_install deskfuse-app
echo ready >> "$calls_path"
''')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, ["stop -x deskfuse-app", "stop -x deskfuse-input-service", "ready"])


if __name__ == "__main__":
    unittest.main()
