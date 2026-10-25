//! Gives the app's Windows binary its icon and its version.

use std::env;

fn main() {
    println!("cargo:rerun-if-changed=windows");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let variable = |name: &str| env::var(name).expect("Cargo sets the package's variables");
    // The resource compiler reads backslashes in a string as escapes.
    let icon = format!("{}/windows/icon.ico", variable("CARGO_MANIFEST_DIR")).replace('\\', "/");
    let version = ["MAJOR", "MINOR", "PATCH"].map(|part| variable(&format!("CARGO_PKG_VERSION_{part}"))).join(",");
    let macros =
        [format!("ICON_PATH=\"{icon}\""), format!("VERSION={version},0"), format!("VERSION_TEXT=\"{}\"", variable("CARGO_PKG_VERSION"))];
    if let Err(error) = embed_resource::compile_for("windows/app.rc", ["bukospeak-desktop"], &macros).manifest_required() {
        panic!("the Windows resource doesn't compile: {error}");
    }
}
