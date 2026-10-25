//! Updates on Windows, where Velopack installs the app: a newer release on GitHub is downloaded
//! in the background and applied on the next start.

use std::thread;

use velopack::sources::GithubSource;
use velopack::{UpdateCheck, UpdateManager, VelopackApp};

/// Does what Velopack starts the app for around an install, an update or an uninstall, and
/// applies an update downloaded before; either ends the process. First in `main`, before
/// anything is started that would have to be stopped.
pub fn start() {
    VelopackApp::build().run();
}

/// Checks for a newer release and downloads it, in the background. A run outside an installation
/// has nothing to update.
pub fn check() {
    let checking = thread::Builder::new().name("update".into()).spawn(|| {
        let source = GithubSource::new(env!("CARGO_PKG_REPOSITORY"), None, false);
        let manager = match UpdateManager::new(source, None, None) {
            Ok(manager) => manager,
            Err(velopack::Error::NotInstalled(_)) => return,
            Err(error) => {
                tracing::warn!("no updates: {error}");
                return;
            }
        };
        match manager.check_for_updates() {
            Ok(UpdateCheck::UpdateAvailable(update)) => {
                if let Err(error) = manager.download_updates(&update, None) {
                    tracing::warn!("the update isn't downloaded: {error}");
                }
            }
            Ok(UpdateCheck::NoUpdateAvailable | UpdateCheck::RemoteIsEmpty) => {}
            Err(error) => tracing::warn!("no updates: {error}"),
        }
    });
    if let Err(error) = checking {
        tracing::warn!("no updates: {error}");
    }
}
