//! What the app does on its own, so it stays out of the way: it joins the call when a game
//! starts, leaves a call one has been alone in a while without a game, and sounds what happens
//! in the call, for while its window isn't looked at.

use std::time::{Duration, Instant};

use bukospeak_core::proto::MemberId;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext, Context, Entity, Subscription, Task};

use crate::audio::Direction;
use crate::call::{Call, Phase};
use crate::cues::{self, Cue, Cues};
use crate::games::Watcher;
use crate::ui::Client;

/// How long one is left alone in a call, with no game running, before the app leaves it.
const ALONE: Duration = Duration::from_secs(5 * 60);

/// How often the app looks whether that time is up, besides whenever the call changes.
const TICK: Duration = Duration::from_secs(15);

/// What the rules have the app do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Do {
    Join,
    Leave,
}

/// What the rules look at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seen {
    /// Whether a game of the settings runs.
    game: bool,
    /// Whether one is out of the call, not joining it either.
    out: bool,
    /// Whether one is alone in a call that is up.
    alone: bool,
    /// Whether there is all that joining takes.
    ready: bool,
}

/// When to join and leave. A game starting asks for a join, which happens once one is out of
/// the call and all is there for it; a join one leaves, or a game running when one joined
/// another way, isn't asked for again until a game starts anew.
#[derive(Debug, Default)]
struct Rules {
    game: bool,
    /// A game started, and the join it asks for hasn't happened.
    asked: bool,
    /// Since when one is alone in the call with no game running.
    alone_since: Option<Instant>,
}

impl Rules {
    fn step(&mut self, now: Instant, seen: Seen) -> Option<Do> {
        self.asked = seen.game && (self.asked || !self.game) && seen.out;
        self.game = seen.game;
        if self.asked && seen.ready {
            self.asked = false;
            return Some(Do::Join);
        }
        if !seen.alone || seen.game {
            self.alone_since = None;
            return None;
        }
        let since = *self.alone_since.get_or_insert(now);
        (now - since >= ALONE).then(|| {
            self.alone_since = None;
            Do::Leave
        })
    }
}

/// What the cues follow of the call.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Snapshot {
    /// Whether a call is up, joining again included.
    in_call: bool,
    /// Whether it is connected, so who is in it is known.
    connected: bool,
    /// Who else is in it.
    others: Vec<MemberId>,
    muted: bool,
}

impl Snapshot {
    fn of(call: &Call) -> Self {
        let you = call.you();
        Self {
            in_call: call.is_in_call(),
            connected: *call.phase() == Phase::Connected,
            others: call.members().iter().map(|member| member.id).filter(|id| Some(*id) != you).collect(),
            muted: call.is_muted(),
        }
    }
}

/// The cues for what changed from `before` to `after`.
fn changes(before: &Snapshot, after: &Snapshot) -> Vec<Cue> {
    let mut cues = Vec::new();
    match (before.in_call, after.in_call) {
        (false, true) => cues.push(Cue::Joined),
        (true, false) => cues.push(Cue::Left),
        // Who comes and goes is known only while connected: joining again, the call is empty.
        _ if before.connected && after.connected => {
            if after.others.iter().any(|id| !before.others.contains(id)) {
                cues.push(Cue::Came);
            }
            if before.others.iter().any(|id| !after.others.contains(id)) {
                cues.push(Cue::Went);
            }
        }
        _ => {}
    }
    match (before.muted, after.muted) {
        (false, true) => cues.push(Cue::Muted),
        (true, false) => cues.push(Cue::Unmuted),
        _ => {}
    }
    cues
}

pub struct Autopilot {
    client: Entity<Client>,
    rules: Rules,
    /// Whether a game of the settings runs, as the watcher told last.
    game: bool,
    /// The games watched for, as the settings had them.
    games: Vec<String>,
    /// The call as the cues followed it last.
    seen: Snapshot,
    /// None if its thread didn't start: then no game is seen running.
    watcher: Option<Watcher>,
    /// None if its thread didn't start: then the app is silent.
    cues: Option<Cues>,
    _client: Subscription,
    _games: Task<()>,
    _ticking: Task<()>,
}

impl Autopilot {
    /// Starts following `client`.
    pub fn new(client: Entity<Client>, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            let (tell, mut told) = mpsc::unbounded();
            let games = client.read(cx).settings.games.clone();
            let watcher = Watcher::new(games.clone(), move |runs| {
                // The app is quitting once nothing listens.
                let _ = tell.unbounded_send(runs);
            })
            .inspect_err(|error| tracing::warn!("no games watched: {error}"))
            .ok();
            let cues = Cues::new(cues::folder()).inspect_err(|error| tracing::warn!("no sounds: {error}")).ok();
            let following = cx.spawn(async move |this, cx| {
                while let Some(runs) = told.next().await {
                    let applied = this.update(cx, |autopilot: &mut Self, cx| {
                        autopilot.game = runs;
                        autopilot.decide(cx);
                    });
                    if applied.is_err() {
                        break;
                    }
                }
            });
            let ticking = cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(TICK).await;
                    if this.update(cx, |autopilot: &mut Self, cx| autopilot.decide(cx)).is_err() {
                        break;
                    }
                }
            });
            Self {
                rules: Rules::default(),
                game: false,
                games,
                seen: Snapshot::of(&client.read(cx).call),
                watcher,
                cues,
                _client: cx.observe(&client, |autopilot, _, cx| autopilot.follow(cx)),
                client,
                _games: following,
                _ticking: ticking,
            }
        })
    }

    /// Sounds what changed in the call, watches for the games the settings have now, and
    /// decides anew.
    fn follow(&mut self, cx: &mut Context<Self>) {
        let client = self.client.read(cx);
        let now = Snapshot::of(&client.call);
        if let Some(cues) = &self.cues {
            for cue in changes(&self.seen, &now) {
                cues.play(cue, client.settings.device(Direction::Output));
            }
        }
        self.seen = now;
        if client.settings.games != self.games {
            self.games = client.settings.games.clone();
            if let Some(watcher) = &self.watcher {
                watcher.set(self.games.clone());
            }
        }
        self.decide(cx);
    }

    fn decide(&mut self, cx: &mut Context<Self>) {
        let client = self.client.read(cx);
        let call = &client.call;
        let seen = Seen {
            game: self.game,
            out: matches!(call.phase(), Phase::Idle | Phase::Failed(_)),
            alone: call.is_in_call() && call.members().len() <= 1,
            ready: client.settings.is_complete(),
        };
        match self.rules.step(Instant::now(), seen) {
            Some(Do::Join) => self.client.update(cx, |client, cx| client.join(cx)),
            Some(Do::Leave) => self.client.update(cx, |client, cx| client.leave(cx)),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: Seen = Seen { game: false, out: true, alone: false, ready: true };
    const PLAYING: Seen = Seen { game: true, ..OUT };
    const IN_A_GAME_CALL: Seen = Seen { game: true, out: false, alone: false, ready: true };

    #[test]
    fn a_game_starting_joins_once() {
        let (mut rules, now) = (Rules::default(), Instant::now());
        assert_eq!(rules.step(now, OUT), None);
        assert_eq!(rules.step(now, PLAYING), Some(Do::Join));
        assert_eq!(rules.step(now, IN_A_GAME_CALL), None);
        // Left during the game: not joined again until a game starts anew.
        assert_eq!(rules.step(now, PLAYING), None);
        assert_eq!(rules.step(now, OUT), None);
        assert_eq!(rules.step(now, PLAYING), Some(Do::Join));
    }

    #[test]
    fn a_game_running_at_the_start_joins_once_all_is_there() {
        let (mut rules, now) = (Rules::default(), Instant::now());
        assert_eq!(rules.step(now, Seen { ready: false, ..PLAYING }), None);
        assert_eq!(rules.step(now, PLAYING), Some(Do::Join));
    }

    #[test]
    fn a_game_starting_in_a_call_joins_nothing() {
        let (mut rules, now) = (Rules::default(), Instant::now());
        assert_eq!(rules.step(now, Seen { game: false, ..IN_A_GAME_CALL }), None);
        assert_eq!(rules.step(now, IN_A_GAME_CALL), None);
        assert_eq!(rules.step(now, PLAYING), None);
    }

    #[test]
    fn alone_without_a_game_leaves_after_a_while() {
        let (mut rules, start) = (Rules::default(), Instant::now());
        let alone = Seen { game: false, out: false, alone: true, ready: true };
        assert_eq!(rules.step(start, alone), None);
        assert_eq!(rules.step(start + ALONE - Duration::from_secs(1), alone), None);
        assert_eq!(rules.step(start + ALONE, alone), Some(Do::Leave));
    }

    #[test]
    fn company_or_a_game_keeps_one_in_the_call() {
        let (mut rules, start) = (Rules::default(), Instant::now());
        let alone = Seen { game: false, out: false, alone: true, ready: true };
        assert_eq!(rules.step(start, alone), None);
        assert_eq!(rules.step(start + ALONE / 2, Seen { alone: false, ..alone }), None);
        assert_eq!(rules.step(start + ALONE, alone), None);
        assert_eq!(rules.step(start + ALONE * 2, Seen { game: true, ..alone }), None);
        assert_eq!(rules.step(start + ALONE * 3, alone), None);
        assert_eq!(rules.step(start + ALONE * 4, alone), Some(Do::Leave));
    }

    fn snapshot(in_call: bool, others: &[u16], muted: bool) -> Snapshot {
        Snapshot { in_call, connected: in_call, others: others.iter().map(|&id| MemberId(id)).collect(), muted }
    }

    #[test]
    fn the_cues_tell_what_changed() {
        let out = snapshot(false, &[], false);
        let with_one = snapshot(true, &[2], false);
        assert_eq!(changes(&out, &with_one), [Cue::Joined]);
        assert_eq!(changes(&with_one, &out), [Cue::Left]);
        assert_eq!(changes(&snapshot(true, &[], false), &with_one), [Cue::Came]);
        assert_eq!(changes(&with_one, &snapshot(true, &[], false)), [Cue::Went]);
        assert_eq!(changes(&with_one, &snapshot(true, &[2], true)), [Cue::Muted]);
        assert_eq!(changes(&with_one, &with_one), []);
    }

    #[test]
    fn joining_again_tells_no_one_coming_or_going() {
        let connected = snapshot(true, &[2], false);
        let reconnecting = Snapshot { connected: false, others: Vec::new(), ..connected.clone() };
        assert_eq!(changes(&connected, &reconnecting), []);
        assert_eq!(changes(&reconnecting, &connected), []);
    }
}
