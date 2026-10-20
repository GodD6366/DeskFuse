"""Exercise the privileged driver branches with all system effects sandboxed."""
import os
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


def make_driver(path: Path, bundle_id: str) -> Path:
    contents = path / 'Contents'
    contents.mkdir(parents=True)
    with (contents / 'Info.plist').open('wb') as stream:
        plistlib.dump({'CFBundleIdentifier': bundle_id}, stream)
    return path


class MicrophoneDriverSandbox:
    """把 manage-driver.sh 复制到临时目录，替换掉特权边界与系统命令。"""

    def __init__(self, root: Path, killall_exit=0):
        self.root = root
        self.hal = root / 'HAL'
        self.hal.mkdir()
        (root / 'compatibility.sh').write_text(
            'MICROPHONE_BUNDLE_NAME=testMicrophone\n'
            'MICROPHONE_BUNDLE_ID=test.microphone\n'
            'LEGACY_MICROPHONE_BUNDLE_NAME=oldMicrophone\n'
            'LEGACY_MICROPHONE_BUNDLE_ID=old.microphone\n'
        )
        self.command_log = root / 'commands'
        killall = root / 'killall'
        killall.write_text(f'#!/bin/bash\nprintf "%s\\n" "$*" >> "{self.command_log}"\nexit {killall_exit}\n')
        killall.chmod(0o755)
        source = (ROOT / 'resources/microphone/manage-driver.sh').read_text()
        # Replace privileged boundaries only in this disposable test copy.
        source = source.replace('[[ "$EUID" == 0 ]]', 'true')
        source = source.replace('HAL=/Library/Audio/Plug-Ins/HAL', f'HAL="{self.hal}"')
        source = source.replace('/usr/bin/killall', f'"{killall}"')
        source = source.replace('/usr/bin/codesign', 'true')
        source = source.replace('/usr/sbin/chown', 'true')
        self.script = root / 'manage-driver.sh'
        self.script.write_text(source)

    def run(self, *args):
        return subprocess.run(
            ['bash', str(self.script), *args],
            capture_output=True, text=True, timeout=5,
            env={**os.environ, 'PATH': '/usr/bin:/bin'},
        )


class MicrophoneDriverRestartTests(unittest.TestCase):
    def run_restart(self, exit_code=0):
        with tempfile.TemporaryDirectory() as directory:
            sandbox = MicrophoneDriverSandbox(Path(directory), killall_exit=exit_code)
            driver = sandbox.hal / 'testMicrophone.driver'
            driver.mkdir()
            sentinel = driver / 'unchanged'
            sentinel.write_text('installed driver')
            result = sandbox.run('restart-audio', '/nonexistent/driver', 'restart-audio')
            self.assertEqual(sentinel.read_text(), 'installed driver')
            self.assertEqual(list(sandbox.hal.iterdir()), [driver])
            self.assertEqual(sandbox.command_log.read_text(), 'coreaudiod\n')
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


class MicrophoneLegacyDriverTests(unittest.TestCase):
    def test_install_replaces_verified_legacy_driver(self):
        with tempfile.TemporaryDirectory() as directory:
            sandbox = MicrophoneDriverSandbox(Path(directory))
            source = make_driver(sandbox.root / 'payload' / 'testMicrophone.driver', 'test.microphone')
            legacy = make_driver(sandbox.hal / 'oldMicrophone.driver', 'old.microphone')
            result = sandbox.run('install', str(source))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue((sandbox.hal / 'testMicrophone.driver/Contents/Info.plist').is_file())
            self.assertFalse(legacy.exists())
            self.assertIn('已移除旧版麦克风驱动', result.stdout)

    def test_install_makes_private_source_readable_by_audio_host(self):
        with tempfile.TemporaryDirectory() as directory:
            sandbox = MicrophoneDriverSandbox(Path(directory))
            source = make_driver(sandbox.root / 'payload' / 'testMicrophone.driver', 'test.microphone')
            binary = source / 'Contents/MacOS/testMicrophone'
            binary.parent.mkdir()
            binary.write_text('binary')
            for path, mode in ((source, 0o700), (source / 'Contents', 0o700), (binary.parent, 0o700),
                               (binary, 0o700), (source / 'Contents/Info.plist', 0o600)):
                path.chmod(mode)
            result = sandbox.run('install', str(source))
            self.assertEqual(result.returncode, 0, result.stderr)
            installed = sandbox.hal / 'testMicrophone.driver'
            modes = {path.relative_to(installed).as_posix(): path.stat().st_mode & 0o777
                     for path in [installed, *installed.rglob('*')]}
            self.assertEqual(modes, {
                '.': 0o755, 'Contents': 0o755, 'Contents/Info.plist': 0o644,
                'Contents/MacOS': 0o755, 'Contents/MacOS/testMicrophone': 0o755,
            })

    def test_uninstall_removes_current_and_legacy_drivers(self):
        with tempfile.TemporaryDirectory() as directory:
            sandbox = MicrophoneDriverSandbox(Path(directory))
            current = make_driver(sandbox.hal / 'testMicrophone.driver', 'test.microphone')
            legacy = make_driver(sandbox.hal / 'oldMicrophone.driver', 'old.microphone')
            result = sandbox.run('uninstall')
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(current.exists())
            self.assertFalse(legacy.exists())

    def test_uninstall_cleans_legacy_only_installation(self):
        with tempfile.TemporaryDirectory() as directory:
            sandbox = MicrophoneDriverSandbox(Path(directory))
            legacy = make_driver(sandbox.hal / 'oldMicrophone.driver', 'old.microphone')
            result = sandbox.run('uninstall')
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(legacy.exists())

    def test_legacy_location_with_foreign_identity_is_kept(self):
        with tempfile.TemporaryDirectory() as directory:
            sandbox = MicrophoneDriverSandbox(Path(directory))
            foreign = make_driver(sandbox.hal / 'oldMicrophone.driver', 'someone.else.driver')
            result = sandbox.run('uninstall')
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(foreign.exists())
            self.assertIn('身份不符，已保留', result.stderr)


if __name__ == '__main__':
    unittest.main()
