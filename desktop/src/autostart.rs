//! Starting the app with the system, in the background: Windows' Run key, a launch agent on
//! macOS, XDG's autostart on Linux, each for the user, starting the program that runs now with
//! [`BACKGROUND`].

use auto_launch::{AutoLaunch, AutoLaunchBuilder, LinuxLaunchMode, MacOSLaunchMode, WindowsEnableMode};

/// What the app is started with by the system: into the tray, its window not shown.
pub const BACKGROUND: &str = "--background";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the program's path: {0}")]
    Path(#[from] std::io::Error),
    #[error(transparent)]
    Launch(#[from] auto_launch::Error),
}

/// The system's start of the program running now.
fn entry() -> Result<AutoLaunch, Error> {
    let program = std::env::current_exe()?;
    Ok(AutoLaunchBuilder::new()
        .set_app_name("BukoSpeak")
        .set_app_path(&program.to_string_lossy())
        .set_args(&[BACKGROUND])
        .set_windows_enable_mode(WindowsEnableMode::CurrentUser)
        .set_macos_launch_mode(MacOSLaunchMode::LaunchAgent)
        .set_linux_launch_mode(LinuxLaunchMode::XdgAutostart)
        .build()?)
}

/// Has the system start the app with it, or not; a start set up for another path, of an older
/// install, is replaced.
pub fn set(on: bool) -> Result<(), Error> {
    let entry = entry()?;
    if on {
        entry.enable()?;
    } else if entry.is_enabled()? {
        entry.disable()?;
    }
    Ok(())
}
