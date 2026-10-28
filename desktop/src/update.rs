//! Updates from the releases on GitHub, for an app Velopack installed (on Windows): a newer
//! release is downloaded as soon as it is found and installed once no call is up, the app
//! starting again in it. At the start the window waits for the first look, so a release found
//! then is installed before anything else.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use bukospeak_desktop::call::Phase;
use futures::StreamExt;
use gpui_kit::{App, AppContext, AsyncApp, Context, Entity, Subscription, Task, WeakEntity};
use velopack::sources::GithubSource;
use velopack::{UpdateCheck, UpdateInfo, UpdateManager, VelopackApp};

use crate::ui::Client;

/// How long the app waits between looks for a newer release, while it runs.
const EVERY: Duration = Duration::from_secs(60 * 60);

/// Does what Velopack starts the app for around an install, an update or an uninstall, and
/// applies an update downloaded before; either ends the process. First in `main`, before
/// anything is started that would have to be stopped.
pub fn start() {
    VelopackApp::build().run();
}

/// Where the updater is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Looking for a newer release.
    Checking,
    /// Nothing newer: none was found, or looking or downloading failed.
    Current,
    /// Downloading `version`, `percent` of it so far.
    Downloading { version: String, percent: u8 },
    /// `version` is downloaded, and installed as soon as no call is up.
    Ready { version: String },
    /// Installing `version`: the app quits, and starts again in it.
    Installing { version: String },
}

pub struct Updater {
    manager: UpdateManager,
    client: Entity<Client>,
    /// The version installed.
    current: String,
    state: State,
    /// The release downloaded, while it waits to be installed.
    downloaded: Option<UpdateInfo>,
    _looking: Task<()>,
    _client: Subscription,
}

impl Updater {
    /// The updater of the installed app, already looking for a newer release; outside an
    /// installation there is nothing to update.
    pub fn new(client: Entity<Client>, cx: &mut App) -> Option<Entity<Self>> {
        let source = GithubSource::new(env!("CARGO_PKG_REPOSITORY"), None, false);
        let manager = match UpdateManager::new(source, None, None) {
            Ok(manager) => manager,
            Err(velopack::Error::NotInstalled(_)) => return None,
            Err(error) => {
                tracing::warn!("no updates: {error}");
                return None;
            }
        };
        Some(cx.new(|cx: &mut Context<Self>| {
            let observed = cx.observe(&client, |updater, _, cx| updater.install_when_due(cx));
            let looking = cx.spawn(async move |updater, cx| {
                while look(&updater, cx).await.is_ok() {
                    cx.background_executor().timer(EVERY).await;
                }
            });
            Self {
                current: manager.get_current_version_as_string(),
                manager,
                client,
                state: State::Checking,
                downloaded: None,
                _looking: looking,
                _client: observed,
            }
        }))
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    /// The version installed.
    pub fn current(&self) -> &str {
        &self.current
    }

    fn set(&mut self, state: State, cx: &mut Context<Self>) {
        self.state = state;
        cx.notify();
    }

    /// Installs the release downloaded: the app quits, a call in it ending, and starts again in
    /// the release.
    pub fn install(&mut self, cx: &mut Context<Self>) {
        let Some(release) = self.downloaded.take() else {
            return;
        };
        let version = release.TargetFullRelease.Version.clone();
        // Velopack's installer waits for the app to quit, then starts it again.
        match self.manager.wait_exit_then_apply_updates(&release, false, true, Vec::<String>::new()) {
            Ok(()) => {
                self.set(State::Installing { version }, cx);
                cx.quit();
            }
            Err(error) => {
                tracing::warn!("{version} isn't installed: {error}");
                self.set(State::Current, cx);
            }
        }
    }

    fn install_when_due(&mut self, cx: &mut Context<Self>) {
        if matches!(self.state, State::Ready { .. }) && !waits(self.client.read(cx).call.phase()) {
            self.install(cx);
        }
    }
}

/// Whether an update waits for the call: while it is up or coming about.
fn waits(phase: &Phase) -> bool {
    matches!(phase, Phase::Joining | Phase::Connected | Phase::Reconnecting)
}

/// Looks for a newer release, unless one is waiting already, and downloads it; fails only when
/// the updater is gone.
async fn look(updater: &WeakEntity<Updater>, cx: &mut AsyncApp) -> anyhow::Result<()> {
    let manager = updater.update(cx, |updater, cx| {
        matches!(updater.state, State::Checking | State::Current).then(|| {
            updater.set(State::Checking, cx);
            updater.manager.clone()
        })
    })?;
    let Some(manager) = manager else {
        return Ok(());
    };
    let checked = cx.background_spawn({
        let manager = manager.clone();
        async move { manager.check_for_updates() }
    });
    let release = match checked.await {
        Ok(UpdateCheck::UpdateAvailable(release)) => *release,
        Ok(UpdateCheck::NoUpdateAvailable | UpdateCheck::RemoteIsEmpty) => {
            return updater.update(cx, |updater, cx| updater.set(State::Current, cx));
        }
        Err(error) => {
            tracing::warn!("no updates: {error}");
            return updater.update(cx, |updater, cx| updater.set(State::Current, cx));
        }
    };
    let version = release.TargetFullRelease.Version.clone();
    let (percents, mut percent) = futures::channel::mpsc::unbounded();
    let downloaded = cx.background_spawn({
        let release = release.clone();
        async move {
            let (sender, receiver) = mpsc::channel::<i16>();
            thread::scope(|scope| {
                // Velopack tells the progress from the thread downloading, into a channel of its
                // own; the percents end when it is done with it.
                scope.spawn(move || {
                    for progress in receiver {
                        // Closed when the updater is gone, and with it whom the progress concerns.
                        let _ = percents.unbounded_send(progress.clamp(0, 100) as u8);
                    }
                });
                manager.download_updates(&release, Some(sender))
            })
        }
    });
    while let Some(percent) = percent.next().await {
        let version = version.clone();
        updater.update(cx, |updater, cx| updater.set(State::Downloading { version, percent }, cx))?;
    }
    let downloaded = downloaded.await;
    updater.update(cx, |updater, cx| match downloaded {
        Ok(()) => {
            updater.downloaded = Some(release);
            updater.set(State::Ready { version }, cx);
            updater.install_when_due(cx);
        }
        Err(error) => {
            tracing::warn!("{version} isn't downloaded: {error}");
            updater.set(State::Current, cx);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_update_waits_only_for_a_call_up_or_coming_about() {
        assert!(!waits(&Phase::Idle));
        assert!(waits(&Phase::Joining));
        assert!(waits(&Phase::Connected));
        assert!(waits(&Phase::Reconnecting));
        assert!(!waits(&Phase::Failed(bukospeak_desktop::call::Failure::NoAnswer)));
    }
}
