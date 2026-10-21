#!/usr/bin/env python3
"""Build once, then enforce a 60-second process deadline for each Swift test."""
import os
from pathlib import Path
import re
import signal
import subprocess
import sys

root = Path(__file__).resolve().parent.parent
if not os.environ.get("DEVELOPER_DIR"):
    for app in ("Xcode.app", "Xcode-beta.app"):
        developer = Path("/Applications") / app / "Contents/Developer"
        if developer.is_dir():
            os.environ["DEVELOPER_DIR"] = str(developer)
            break
package = str(root / "apps/deskfuse-macos/RemoteApplication")
subprocess.run(["swift", "build", "--package-path", package, "--build-tests"], check=True)
command = ["swift", "test", "--package-path", package, "--skip-build"]
listing = subprocess.check_output(command + ["list"], text=True)
tests = [line.strip().removesuffix("()") for line in listing.splitlines()
         if line.startswith("RemoteApplicationTests.")]
if not tests:
    sys.exit("error: no RemoteApplication tests discovered")
for test in tests:
    print(f"== {test} (60s) ==", flush=True)
    # Swift Testing applies the regex to the suite-qualified identifier but
    # does not accept `^...$` anchors around a slash-qualified suite path.
    process = subprocess.Popen(command + ["--filter", re.escape(test)],
                               start_new_session=True, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, text=True)
    try:
        output, _ = process.communicate(timeout=60)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.communicate()
        sys.exit(f"error: test timed out: {test}")
    print(output, end="")
    # `swift test --filter` exits successfully when nothing matched, so a
    # zero status alone must not be reported as a pass.
    if process.returncode or "No matching test cases" in output:
        sys.exit(process.returncode or f"error: test did not run: {test}")
print(f"RemoteApplication: {len(tests)} isolated tests passed")
