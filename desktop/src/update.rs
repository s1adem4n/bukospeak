use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[cfg(target_os = "windows")]
use crate::autostart;
use crate::autostart::BACKGROUND;
use crate::call::Phase;
use futures::StreamExt;
use gpui::{App, AppContext, AsyncApp, Context, Entity, Subscription, Task, WeakEntity};
use velopack::sources::GithubSource;
use velopack::{UpdateCheck, UpdateInfo, UpdateManager, VelopackApp};

use crate::ui::Client;

const EVERY: Duration = Duration::from_secs(60 * 60);

pub fn start() {
    velopack().run();
}

fn velopack() -> VelopackApp<'static> {
    let app = VelopackApp::build();
    // Velopack installs on Windows only; a start with the system left after it would point at nothing.
    #[cfg(target_os = "windows")]
    let app = app.on_before_uninstall_fast_callback(|_| {
        if let Err(error) = autostart::set(false) {
            tracing::warn!("the start with the system stays: {error}");
        }
    });
    app
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Checking,
    Current,
    Downloading { version: String, percent: u8 },
    Ready { version: String },
    Installing { version: String },
}

pub struct Updater {
    manager: UpdateManager,
    client: Entity<Client>,
    current: String,
    state: State,
    downloaded: Option<UpdateInfo>,
    _looking: Task<()>,
    _client: Subscription,
}

impl Updater {
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

    pub fn current(&self) -> &str {
        &self.current
    }

    fn set(&mut self, state: State, cx: &mut Context<Self>) {
        self.state = state;
        cx.notify();
    }

    pub fn install(&mut self, cx: &mut Context<Self>) {
        let Some(release) = self.downloaded.take() else {
            return;
        };
        let version = release.TargetFullRelease.Version.clone();
        // Velopack starts the app again with these, so one in the background stays there.
        let arguments: &[&str] = if cx.windows().is_empty() { &[BACKGROUND] } else { &[] };
        match self.manager.wait_exit_then_apply_updates(&release, false, true, arguments) {
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

fn waits(phase: &Phase) -> bool {
    matches!(phase, Phase::Joining | Phase::Connected | Phase::Reconnecting)
}

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
                // Velopack sends the progress from its downloading thread into a channel of its own.
                scope.spawn(move || {
                    for progress in receiver {
                        // Closed only once the updater is gone.
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
        assert!(!waits(&Phase::Failed(crate::call::Failure::NoAnswer)));
    }
}
