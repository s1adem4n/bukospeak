"""An install build for the iPhone: a development-signed IPA with an install page in
~/bukospeak-ota on the Mac, which `tailscale serve` shares with the tailnet only. Opening the
page in Safari on the phone installs the app wherever Tailscale is on."""

import json
import tempfile
import time
from pathlib import Path
from string import Template

import mac
import xcode
from config import APP, Target

TEMPLATES = Path(__file__).resolve().parent / "ota"
TAILSCALE = "/Applications/Tailscale.app/Contents/MacOS/Tailscale"
# The Mac serves other apps' install pages too, on 8765 and 8766.
PORT = 8767


def ota(target: Target, configuration: str) -> None:
    """`configuration` is Debug, quick to build incrementally, or Release, optimized and slower to build."""
    xcode.build(target, configuration)
    app = f"build/Build/Products/{configuration}-iphoneos/BukoSpeak.app"
    mac.shell(
        target,
        f"""cd "$dir/ios"
mkdir -p "$HOME/bukospeak-ota"
rm -rf build-ota
mkdir -p build-ota/Payload
cp -R {app} build-ota/Payload/
(cd build-ota && zip -qry BukoSpeak.ipa Payload)
cp build-ota/BukoSpeak.ipa "$HOME/bukospeak-ota/BukoSpeak.ipa"
""",
    )
    name = json.loads(mac.run(target, TAILSCALE, "status", "--json", capture=True).stdout)["Self"]["DNSName"].rstrip(".")
    info = f"{target.dir}/ios/{app}/Info.plist"
    values = {
        "base": f"https://{name}/bukospeak",
        "app": APP,
        "version": mac.run(target, "plutil", "-extract", "CFBundleShortVersionString", "raw", info, capture=True).stdout.strip(),
        # The stamp keeps Safari and the installer from reusing an older download.
        "stamp": time.strftime("%Y%m%d%H%M%S"),
        "built": time.strftime("%-d.%-m. %H:%M"),
        "out": mac.shell(target, 'echo "$HOME/bukospeak-ota"', capture=True).stdout.strip(),
        "port": str(PORT),
    }
    with tempfile.TemporaryDirectory() as folder:
        for template in ("manifest.plist", "index.html", "server.plist"):
            (Path(folder) / template).write_text(Template((TEMPLATES / template).read_text()).substitute(values))
            mac.copy(str(Path(folder) / template), f"{target.host}:bukospeak-ota/{template}")
    _serve(target)
    print(f"{values['base']}/")


def _serve(target: Target) -> None:
    """The App Store Tailscale can't serve files itself, so a small server on localhost does,
    kept running by launchd, and Tailscale forwards the tailnet (only) to it."""
    mac.shell(
        target,
        f"""label=digital.buko.bukospeak-ota
agent="$HOME/Library/LaunchAgents/$label.plist"
if ! launchctl print "gui/$(id -u)/$label" >/dev/null 2>&1; then
  mv "$HOME/bukospeak-ota/server.plist" "$agent"
  launchctl bootstrap "gui/$(id -u)" "$agent"
fi
rm -f "$HOME/bukospeak-ota/server.plist"
{TAILSCALE} serve status | grep -q '/bukospeak ' || {TAILSCALE} serve --bg --set-path /bukospeak http://127.0.0.1:{PORT} >/dev/null
""",
    )
