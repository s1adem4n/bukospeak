"""Which Mac this checkout builds on, and where there."""

import os
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path

IOS = Path(__file__).resolve().parent.parent
REPO = IOS.parent
APP = "digital.buko.BukoSpeak"

# The Mac the iPhone is plugged into, whose tailnet name the install page lives at.
HOST = "macmini"
XCODE = "/Applications/Xcode-26.2.0.app"


@dataclass(frozen=True)
class Target:
    host: str
    xcode: str
    # The checkout's folder on the Mac, relative to its home.
    dir: str

    @property
    def developer_dir(self) -> str:
        return f"{self.xcode}/Contents/Developer"


def _default_dir() -> str:
    # Each checkout builds in its own folder on the Mac, so parallel worktrees never sync over each other.
    top = Path(subprocess.check_output(["git", "-C", str(REPO), "rev-parse", "--show-toplevel"], text=True).strip())
    if top == Path.home() / "dev/bukospeak":
        return "dev/bukospeak"
    return "dev/bukospeak-" + re.sub(r"[^A-Za-z0-9_]", "_", top.name)


def target() -> Target:
    folder = os.environ.get("BUKOSPEAK_MAC_DIR") or _default_dir()
    # Syncing deletes everything else in the folder, so it must be one of ours.
    if not re.fullmatch(r"dev/bukospeak(-[A-Za-z0-9_]+)?", folder):
        raise SystemExit("BUKOSPEAK_MAC_DIR must be dev/bukospeak or dev/bukospeak-<name>")
    return Target(
        host=os.environ.get("BUKOSPEAK_MAC_HOST") or HOST,
        xcode=os.environ.get("BUKOSPEAK_XCODE") or XCODE,
        dir=folder,
    )
