"""The app's screens in staged scenes, as PNGs here: its Debug build on a simulator of the Mac's
own, launched with `-stage <scene>` (BukoSpeak/Harness/Staged.swift)."""

import re
import shlex
import tempfile
from pathlib import Path

import mac
import xcode
from config import APP, IOS, Target

SIMULATOR = "BukoSpeak Stage"
DEVICE = "com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro"
RUNTIME = "com.apple.CoreSimulator.SimRuntime.iOS-26-2"
APPEARANCES = ("dark", "light")
# Long enough for a sheet to have come up and a scene's springs to have settled.
SETTLE = 2.5


def scenes() -> list[str]:
    return re.findall(r'Staged\(name: "([^"]+)"', (IOS / "BukoSpeak/Harness/Staged.swift").read_text())


def _rust_triple(target: Target) -> str:
    machine = mac.run(target, "uname", "-m", capture=True).stdout.strip()
    return "aarch64-apple-ios-sim" if machine == "arm64" else "x86_64-apple-ios"


def snap(target: Target, chosen: list[str], appearances: tuple[str, ...]) -> None:
    known = scenes()
    if unknown := [scene for scene in chosen if scene not in known]:
        mac.fail(f"no scene {', '.join(unknown)}; there are {', '.join(known)}")
    mac.upload(target)
    xcode.rust(target, _rust_triple(target), lint=False)
    xcode.app(target, "Debug", f"platform=iOS Simulator,name={SIMULATOR}", "build-sim")
    shots = []
    for appearance in appearances:
        for scene in chosen or known:
            shots.append(f"""xcrun simctl ui "$sim" appearance {appearance}
xcrun simctl terminate "$sim" {APP} 2>/dev/null || true
xcrun simctl launch "$sim" {APP} -stage {shlex.quote(scene)} >/dev/null
sleep {SETTLE}
xcrun simctl io "$sim" screenshot --type=png "$out/{scene}-{appearance}.png" >/dev/null 2>&1""")
    remote = "bukospeak-snaps/" + Path(target.dir).name
    mac.shell(
        target,
        f"""sim={shlex.quote(SIMULATOR)}
xcrun simctl list devices | grep -q "$sim (" || xcrun simctl create "$sim" {DEVICE} {RUNTIME} >/dev/null
xcrun simctl boot "$sim" 2>/dev/null || true
xcrun simctl bootstatus "$sim" -b >/dev/null
xcrun simctl status_bar "$sim" override --time 9:41 --batteryState charged --batteryLevel 100 --cellularBars 4 --wifiBars 3
xcrun simctl install "$sim" "$dir/ios/build-sim/Build/Products/Debug-iphonesimulator/BukoSpeak.app"
out=$HOME/{remote}
rm -rf "$out" && mkdir -p "$out"
"""
        + "\n".join(shots),
    )
    local = Path(tempfile.gettempdir()) / remote
    mac.fetch_folder(target, remote, local)
    for shot in sorted(local.glob("*.png")):
        print(shot)
