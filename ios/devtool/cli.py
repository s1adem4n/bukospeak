"""ios/dev's commands."""

import sys

import config
import mac
import ota
import stage
import xcode

HELP = """usage: ios/dev <command>

  build                 sync the repository to the Mac, lint and build the Rust library, run swift format
                          lint and build the app for the iPhone, warnings as errors; prints errors and warnings
  format                rewrite BukoSpeak/ with swift format (ios/.swift-format)
  ota [--release]       build an IPA and print the tailnet-only page that installs it on the iPhone;
                          a Debug build by default, quick to rebuild; --release is optimized
  snap [scene…] [--dark|--light]
                        build for the Mac's simulator and screenshot the staged scenes
                          (BukoSpeak/Harness/Staged.swift), all and in both appearances by default
  clean                 remove this checkout's folder on the Mac
  mac 'script'          run bash on the Mac with $dir, the checkout's folder there

Each checkout syncs to its own Mac folder (dev/bukospeak-<worktree>); BUKOSPEAK_MAC_DIR overrides it.
BUKOSPEAK_MAC_HOST and BUKOSPEAK_XCODE override the Mac and its Xcode."""


def clean(target: config.Target) -> None:
    if target.dir == "dev/bukospeak":
        mac.fail("the main checkout's folder stays")
    mac.run(target, "rm", "-rf", target.dir)
    print(f"removed {target.host}:{target.dir}")


def main() -> None:
    command, *args = sys.argv[1:] or ["help"]
    if command in ("help", "-h", "--help"):
        return print(HELP)

    target = config.target()
    match command, args:
        case "build", []:
            xcode.build(target)
        case "format", []:
            xcode.format_files(target)
        case "ota", [] | ["--release"]:
            ota.ota(target, "Release" if args else "Debug")
        case "snap", _:
            appearances = tuple(appearance for appearance in stage.APPEARANCES if f"--{appearance}" in args)
            scenes = [arg for arg in args if not arg.startswith("--")]
            stage.snap(target, scenes, appearances or stage.APPEARANCES)
        case "clean", []:
            clean(target)
        case "mac", [script]:
            mac.shell(target, script)
        case _:
            mac.fail(HELP)
