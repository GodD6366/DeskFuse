"""Select public source paths without reading ignored local files or following links."""
from pathlib import Path
import os
import subprocess

ROOT = Path(__file__).resolve().parent.parent


def source_files(*, worktree: bool = False) -> set[str]:
    args = ["git", "ls-files", "--cached", "-z"]
    if worktree:
        args.extend(["--others", "--exclude-standard"])
    names = {os.fsdecode(raw) for raw in subprocess.check_output(args, cwd=ROOT).split(b"\0") if raw}
    # A pending deletion is excluded only when reviewing the effective worktree.
    # lexists retains broken symlinks so the caller can reject them, not follow them.
    return {name for name in names if not worktree or os.path.lexists(ROOT / name)}
