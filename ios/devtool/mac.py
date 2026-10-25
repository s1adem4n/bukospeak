"""Commands on the Mac over one reused ssh connection, and files to and from it.

The logic lives here on this machine; the Mac runs single commands and needs only Xcode, mise
and Tailscale.
"""

import shlex
import subprocess
import sys
from pathlib import Path

from config import REPO, Target

_SSH = ["ssh", "-o", "ControlMaster=auto", "-o", "ControlPath=/tmp/bukospeak-ssh-%r@%h", "-o", "ControlPersist=10m"]


def _environment(target: Target) -> str:
    return f"""export DEVELOPER_DIR={shlex.quote(target.developer_dir)}
export PATH=$HOME/.local/bin:/usr/local/bin:/opt/homebrew/bin:$PATH
dir=$HOME/{target.dir}
export MISE_TRUSTED_CONFIG_PATHS=$dir
"""


def shell(target: Target, script: str, *, capture: bool = False, check: bool = True) -> subprocess.CompletedProcess:
    """Runs a bash script on the Mac with $dir set.

    Output streams through unless `capture`. A failure exits with its status unless `check` is off.
    """
    # The Mac's login shell is fish; the quoted script reaches bash whole.
    command = shlex.join(["bash", "-c", "set -euo pipefail\n" + _environment(target) + script])
    result = subprocess.run(
        [*_SSH, target.host, command],
        text=True,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
        check=False,
    )
    if check and result.returncode != 0:
        if capture:
            sys.stderr.write(result.stderr)
        raise SystemExit(result.returncode)
    return result


def run(target: Target, *argv: str, capture: bool = False, check: bool = True) -> subprocess.CompletedProcess:
    """Runs one command on the Mac, its arguments quoted; $dir isn't expanded."""
    return shell(target, shlex.join(argv), capture=capture, check=check)


def stream(target: Target, script: str) -> subprocess.Popen:
    """Starts a script whose output, stdout and stderr together, the caller reads line by line."""
    command = shlex.join(["bash", "-c", "set -euo pipefail\n" + _environment(target) + script])
    return subprocess.Popen([*_SSH, target.host, command], text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)


def upload(target: Target) -> None:
    """Copies the repository to the Mac, keeping its Rust and Xcode build folders.

    What git ignores stays here: it holds the server's key and other secrets, besides builds.
    """
    subprocess.run(
        [
            "rsync",
            "-a",
            "--delete",
            "-e",
            shlex.join(_SSH),
            "--filter=:- .gitignore",
            "--exclude=.git",
            f"{REPO}/",
            f"{target.host}:{target.dir}/",
        ],
        check=True,
    )


def copy(source: str, destination: str) -> None:
    """scp over the shared connection; the Mac's side is written `host:path`."""
    subprocess.run(["scp", "-q", *_SSH[1:], source, destination], check=True)


def fetch_folder(target: Target, remote: str, local: Path) -> None:
    """Copies a folder's files from the Mac in one stream."""
    local.mkdir(parents=True, exist_ok=True)
    packed = subprocess.run(
        [*_SSH, target.host, shlex.join(["tar", "--no-mac-metadata", "-C", remote, "-cf", "-", "."])],
        stdout=subprocess.PIPE,
        check=True,
    )
    subprocess.run(["tar", "--warning=no-unknown-keyword", "-C", str(local), "-xf", "-"], input=packed.stdout, check=True)


def fail(message: str, status: int = 2):
    print(message, file=sys.stderr)
    raise SystemExit(status)
