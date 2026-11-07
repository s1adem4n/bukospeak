use auto_launch::{AutoLaunch, AutoLaunchBuilder, LinuxLaunchMode, MacOSLaunchMode, WindowsEnableMode};

pub const BACKGROUND: &str = "--background";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the program's path: {0}")]
    Path(#[from] std::io::Error),
    #[error(transparent)]
    Launch(#[from] auto_launch::Error),
}

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

pub fn set(on: bool) -> Result<(), Error> {
    let entry = entry()?;
    if on {
        entry.enable()?;
    } else if entry.is_enabled()? {
        entry.disable()?;
    }
    Ok(())
}
