"""Exercise the privileged restart branch with all system effects sandboxed."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class MicrophoneDriverRestartTests(unittest.TestCase):
    def run_restart(self, exit_code=0):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            hal = root / 'HAL'
            driver = hal / 'testMicrophone.driver'
            driver.mkdir(parents=True)
            sentinel = driver / 'unchanged'
            sentinel.write_text('installed driver')
            (root / 'compatibility.sh').write_text(
                'MICROPHONE_BUNDLE_NAME=testMicrophone\n'
                'MICROPHONE_BUNDLE_ID=test.microphone\n'
            )
            command_log = root / 'commands'
            killall = root / 'killall'
            killall.write_text(f'#!/bin/bash\nprintf "%s\\n" "$*" >> "{command_log}"\nexit {exit_code}\n')
            killall.chmod(0o755)
            source = (ROOT / 'resources/microphone/manage-driver.sh').read_text()
            # Replace privileged boundaries only in this disposable test copy.
            source = source.replace('[[ "$EUID" == 0 ]]', 'true')
            source = source.replace('HAL=/Library/Audio/Plug-Ins/HAL', f'HAL="{hal}"')
            source = source.replace('/usr/bin/killall', f'"{killall}"')
            script = root / 'manage-driver.sh'
            script.write_text(source)
            result = subprocess.run(
                ['bash', str(script), 'restart-audio', '/nonexistent/driver', 'restart-audio'],
                capture_output=True, text=True, timeout=5,
                env={**os.environ, 'PATH': '/usr/bin:/bin'},
            )
            self.assertEqual(sentinel.read_text(), 'installed driver')
            self.assertEqual(list(hal.iterdir()), [driver])
            self.assertEqual(command_log.read_text(), 'coreaudiod\n')
            return result

    def test_restart_needs_no_source_and_keeps_driver_files(self):
        result = self.run_restart()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('已请求重启音频服务', result.stdout)
        self.assertNotIn('安装完成', result.stdout)

    def test_restart_failure_is_reported_without_install_success(self):
        result = self.run_restart(exit_code=1)
        self.assertEqual(result.returncode, 1)
        self.assertIn('无法重启音频服务', result.stderr)
        self.assertNotIn('已请求重启', result.stdout)
        self.assertNotIn('驱动文件操作已完成', result.stderr)


if __name__ == '__main__':
    unittest.main()
