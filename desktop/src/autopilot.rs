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

const ALONE: Duration = Duration::from_secs(5 * 60);

const TICK: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Do {
    Join,
    Leave,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Seen {
    game_runs: bool,
    out_of_call: bool,
    alone: bool,
    can_join: bool,
}

#[derive(Debug, Default)]
struct Rules {
    game_ran: bool,
    join_asked: bool,
    alone_since: Option<Instant>,
}

impl Rules {
    fn step(&mut self, now: Instant, seen: Seen) -> Option<Do> {
        self.join_asked = seen.game_runs && (self.join_asked || !self.game_ran) && seen.out_of_call;
        self.game_ran = seen.game_runs;
        if self.join_asked && seen.can_join {
            self.join_asked = false;
            return Some(Do::Join);
        }
        if !seen.alone || seen.game_runs {
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Snapshot {
    active: bool,
    in_call: bool,
    connected: bool,
    others: Vec<MemberId>,
    muted: bool,
}

impl Snapshot {
    fn of(call: &Call) -> Self {
        let you = call.you();
        Self {
            active: matches!(call.phase(), Phase::Joining | Phase::Connected | Phase::Reconnecting),
            in_call: call.is_in_call(),
            connected: *call.phase() == Phase::Connected,
            others: call.members().iter().map(|member| member.id).filter(|id| Some(*id) != you).collect(),
            muted: call.is_muted(),
        }
    }
}

fn changes(before: &Snapshot, after: &Snapshot) -> Vec<Cue> {
    let mut cues = Vec::new();
    match (before.in_call, after.in_call) {
        (false, true) => cues.push(Cue::Joined),
        (true, false) => cues.push(Cue::Left),
        // Who is in the call is known only while connected; joining again, it is empty.
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
    game_runs: bool,
    games: Vec<String>,
    seen: Snapshot,
    watcher: Option<Watcher>,
    cues: Option<Cues>,
    _client: Subscription,
    _games: Task<()>,
    _ticking: Task<()>,
}

impl Autopilot {
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
                        autopilot.game_runs = runs;
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
                game_runs: false,
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

    fn follow(&mut self, cx: &mut Context<Self>) {
        let client = self.client.read(cx);
        let now = Snapshot::of(&client.call);
        if let Some(cues) = &self.cues {
            let output = client.settings.device(Direction::Output);
            // Open from the start of a join, so the device is awake by the time its cue plays.
            if now.active && !self.seen.active {
                cues.open(output.clone());
            }
            let changed = changes(&self.seen, &now);
            if !changed.is_empty() {
                cues.play(changed, output.clone());
            }
            if !now.active && self.seen.active {
                cues.close();
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
            game_runs: self.game_runs,
            out_of_call: matches!(call.phase(), Phase::Idle | Phase::Failed(_)),
            alone: call.is_in_call() && call.members().len() <= 1,
            can_join: client.settings.is_complete(),
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

    const OUT: Seen = Seen { game_runs: false, out_of_call: true, alone: false, can_join: true };
    const PLAYING: Seen = Seen { game_runs: true, ..OUT };
    const IN_A_GAME_CALL: Seen = Seen { game_runs: true, out_of_call: false, alone: false, can_join: true };

    #[test]
    fn a_game_starting_joins_once() {
        let (mut rules, now) = (Rules::default(), Instant::now());
        assert_eq!(rules.step(now, OUT), None);
        assert_eq!(rules.step(now, PLAYING), Some(Do::Join));
        assert_eq!(rules.step(now, IN_A_GAME_CALL), None);
        assert_eq!(rules.step(now, PLAYING), None);
        assert_eq!(rules.step(now, OUT), None);
        assert_eq!(rules.step(now, PLAYING), Some(Do::Join));
    }

    #[test]
    fn a_game_running_at_the_start_joins_once_all_is_there() {
        let (mut rules, now) = (Rules::default(), Instant::now());
        assert_eq!(rules.step(now, Seen { can_join: false, ..PLAYING }), None);
        assert_eq!(rules.step(now, PLAYING), Some(Do::Join));
    }

    #[test]
    fn a_game_starting_in_a_call_joins_nothing() {
        let (mut rules, now) = (Rules::default(), Instant::now());
        assert_eq!(rules.step(now, Seen { game_runs: false, ..IN_A_GAME_CALL }), None);
        assert_eq!(rules.step(now, IN_A_GAME_CALL), None);
        assert_eq!(rules.step(now, PLAYING), None);
    }

    #[test]
    fn alone_without_a_game_leaves_after_a_while() {
        let (mut rules, start) = (Rules::default(), Instant::now());
        let alone = Seen { game_runs: false, out_of_call: false, alone: true, can_join: true };
        assert_eq!(rules.step(start, alone), None);
        assert_eq!(rules.step(start + ALONE - Duration::from_secs(1), alone), None);
        assert_eq!(rules.step(start + ALONE, alone), Some(Do::Leave));
    }

    #[test]
    fn company_or_a_game_keeps_one_in_the_call() {
        let (mut rules, start) = (Rules::default(), Instant::now());
        let alone = Seen { game_runs: false, out_of_call: false, alone: true, can_join: true };
        assert_eq!(rules.step(start, alone), None);
        assert_eq!(rules.step(start + ALONE / 2, Seen { alone: false, ..alone }), None);
        assert_eq!(rules.step(start + ALONE, alone), None);
        assert_eq!(rules.step(start + ALONE * 2, Seen { game_runs: true, ..alone }), None);
        assert_eq!(rules.step(start + ALONE * 3, alone), None);
        assert_eq!(rules.step(start + ALONE * 4, alone), Some(Do::Leave));
    }

    fn snapshot(in_call: bool, others: &[u16], muted: bool) -> Snapshot {
        Snapshot { active: in_call, in_call, connected: in_call, others: others.iter().map(|&id| MemberId(id)).collect(), muted }
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
