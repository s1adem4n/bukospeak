"""Building on the Mac: the Rust library, then the app."""

import re
import time
from pathlib import Path

import mac
from config import IOS, Target

RUST_TARGET = "aarch64-apple-ios"

# swift-format runs from Xcode's toolchain, as the build does.
SWIFT_FORMAT = 'cd "$dir/ios" && xcrun swift-format {action} --recursive --parallel --configuration .swift-format BukoSpeak'

COMPILED = re.compile(r"^(SwiftCompile|CompileSwift|CompileC|\s+Compiling) ")


def _log(target: Target, name: str) -> Path:
    path = Path("/tmp/bukospeak-build") / Path(target.dir).name / f"{name}.log"
    path.parent.mkdir(parents=True, exist_ok=True)
    return path


def _relative(target: Target, line: str) -> str:
    return re.sub(rf"/Users/[^/]+/{re.escape(target.dir)}/", "", line)


def _streamed(target: Target, name: str, script: str) -> tuple[int, list[str]]:
    """Runs a script on the Mac and returns its status and output. The output goes to a log
    here, and a line every 30 s keeps a long build from being silent."""
    process = mac.stream(target, script)
    lines: list[str] = []
    compiled = 0
    start = time.monotonic()
    next_report = 30.0
    with _log(target, name).open("w") as log:
        assert process.stdout
        for line in process.stdout:
            log.write(line)
            lines.append(line.rstrip("\n"))
            compiled += bool(COMPILED.match(line))
            elapsed = time.monotonic() - start
            if elapsed >= next_report:
                next_report = elapsed + 30
                print(f"… {int(elapsed // 60)}m{int(elapsed % 60)}s {name}: {compiled} compiled", flush=True)
    return process.wait(), lines


def rust(target: Target) -> None:
    """Lints and builds the C interface's static library for the iPhone, always optimized:
    noise suppression and Opus unoptimized are too slow for the realtime callbacks. Its iOS
    code only builds on the Mac, whose SDK the C dependencies need."""
    flags = f"--target {RUST_TARGET} --release --locked"
    status, lines = _streamed(
        target,
        "rust",
        f"""cd "$dir"
mise install --quiet rust
export IPHONEOS_DEPLOYMENT_TARGET=26.0
mise exec -- cargo clippy -p bukospeak-ffi -p bukospeak-audio-ios {flags} -- -D warnings
mise exec -- cargo build -p bukospeak-ffi {flags}
""",
    )
    if status:
        print("\n".join(_relative(target, line) for line in lines if re.match(r"^(error|warning)", line) or line.startswith("  -->")))
        mac.fail(f"rust failed (full log: {_log(target, 'rust')})", status)


def lint(target: Target) -> None:
    """Fails on what `swift format lint --strict` finds, before a build spends minutes."""
    linted = mac.shell(target, SWIFT_FORMAT.format(action="lint --strict") + " 2>&1", capture=True, check=False)
    if linted.returncode:
        findings = [_relative(target, line) for line in linted.stdout.splitlines() if line.strip()]
        print("\n".join(findings[:40]))
        mac.fail(f"swift format: {len(findings)} findings; ios/dev format rewrites the files", 1)


def format_files(target: Target) -> None:
    """Rewrites BukoSpeak/ with swift format on the Mac and brings it back."""
    mac.upload(target)
    mac.shell(target, SWIFT_FORMAT.format(action="format --in-place"))
    mac.fetch_folder(target, f"{target.dir}/ios/BukoSpeak", IOS / "BukoSpeak")
    print("formatted BukoSpeak")


def build(target: Target, configuration: str = "Debug") -> None:
    """Syncs the repository, builds the Rust library, lints and builds the app for the iPhone,
    signed, warnings as errors; prints errors and warnings."""
    mac.upload(target)
    rust(target)
    lint(target)
    status, lines = _streamed(
        target,
        "build" if configuration == "Debug" else f"build-{configuration.lower()}",
        f'cd "$dir/ios" && xcodebuild -project BukoSpeak.xcodeproj -scheme BukoSpeak -configuration {configuration}'
        " -destination generic/platform=iOS -derivedDataPath build -allowProvisioningUpdates build",
    )
    issues = {_relative(target, line) for line in lines if re.search(r"error:|warning:", line) and "appintents" not in line}
    for issue in sorted(issues):
        print(issue)
    print("build succeeded" if status == 0 else "build failed")
    if status:
        raise SystemExit(status)
