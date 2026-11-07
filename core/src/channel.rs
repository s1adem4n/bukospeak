use std::{fmt, mem};

use bukospeak_proto::MemberId;
use bukospeak_proto::control::Member;

use crate::session::{self, Error};

#[derive(Debug)]
pub(crate) enum Event {
    Entered { call: u64, you: MemberId, members: Vec<Member> },
    Joined(Member),
    Changed(Member),
    Left(MemberId),
    Speaking(Vec<MemberId>),
    Reconnecting,
    Heard,
    Ended,
    Lost(Lost),
    Present(Vec<Member>),
}

#[derive(Debug, thiserror::Error)]
pub enum Lost {
    #[error("{connection}; joining again failed: {rejoin}")]
    Unrecovered { connection: Error, rejoin: Error },
    #[error("this device joined again elsewhere")]
    Replaced,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Out,
    Connected,
    Reconnecting,
}

#[derive(Clone, Debug, Default)]
pub struct Channel {
    call: u64,
    state: State,
    you: Option<MemberId>,
    members: Vec<Member>,
    speaking: Vec<MemberId>,
    present: Vec<Member>,
}

impl Channel {
    pub fn shown(state: State, you: Option<MemberId>, members: Vec<Member>, speaking: Vec<MemberId>) -> Self {
        Self { call: 0, state, you, members, speaking, present: Vec::new() }
    }

    pub fn call(&self) -> u64 {
        self.call
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn you(&self) -> Option<MemberId> {
        self.you
    }

    pub fn members(&self) -> &[Member] {
        &self.members
    }

    pub fn speaking(&self) -> &[MemberId] {
        &self.speaking
    }

    pub fn present(&self) -> &[Member] {
        &self.present
    }

    pub(crate) fn apply(&mut self, event: &Event) -> Option<Announcement> {
        match event {
            Event::Entered { call, you, members } => {
                let rejoined = self.state == State::Reconnecting;
                let present = mem::take(&mut self.present);
                *self =
                    Self { call: *call, state: State::Connected, you: Some(*you), members: members.clone(), present, ..Self::default() };
                rejoined.then_some(Announcement::Reconnected)
            }
            Event::Joined(member) => {
                self.members.push(member.clone());
                Some(Announcement::Joined(member.name.clone()))
            }
            Event::Changed(member) => {
                session::replace(&mut self.members, member.clone());
                None
            }
            Event::Left(id) => {
                let at = self.members.iter().position(|member| member.id == *id)?;
                self.speaking.retain(|speaker| speaker != id);
                Some(Announcement::Left(self.members.remove(at).name))
            }
            Event::Speaking(now) => {
                self.speaking.clone_from(now);
                None
            }
            Event::Reconnecting => {
                let was = mem::replace(&mut self.state, State::Reconnecting);
                self.speaking.clear();
                (was == State::Connected).then_some(Announcement::Reconnecting)
            }
            Event::Heard => {
                self.state = State::Connected;
                Some(Announcement::Reconnected)
            }
            // The watch may have started again before the leave ended.
            Event::Ended | Event::Lost(_) => {
                *self = Self { call: self.call, present: mem::take(&mut self.present), ..Self::default() };
                None
            }
            Event::Present(members) => {
                self.present.clone_from(members);
                None
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Announcement {
    Joined(String),
    Left(String),
    Reconnecting,
    Reconnected,
}

impl fmt::Display for Announcement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Joined(name) => write!(f, "{name} ist beigetreten"),
            Self::Left(name) => write!(f, "{name} hat den Kanal verlassen"),
            Self::Reconnecting => f.write_str("Verbindung verloren"),
            Self::Reconnected => f.write_str("Wieder verbunden"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: u16, name: &str) -> Member {
        Member { id: MemberId(id), name: name.into(), muted: false, reconnecting: false }
    }

    fn entered(you: u16, members: &[Member]) -> Event {
        Event::Entered { call: 1, you: MemberId(you), members: members.to_vec() }
    }

    fn names(channel: &Channel) -> Vec<&str> {
        channel.members().iter().map(|member| member.name.as_str()).collect()
    }

    #[test]
    fn entering_shows_who_is_there_without_announcing_them() {
        let mut channel = Channel::default();
        assert_eq!(channel.apply(&entered(2, &[member(1, "first"), member(2, "second")])), None);
        assert_eq!(channel.state(), State::Connected);
        assert_eq!(channel.you(), Some(MemberId(2)));
        assert_eq!(names(&channel), ["first", "second"]);
    }

    #[test]
    fn announces_who_comes_and_goes_by_name() {
        let mut channel = Channel::default();
        channel.apply(&entered(1, &[member(1, "first")]));

        let joined = channel.apply(&Event::Joined(member(2, "second"))).unwrap();
        assert_eq!(joined.to_string(), "second ist beigetreten");
        assert_eq!(names(&channel), ["first", "second"]);

        channel.apply(&Event::Speaking(vec![MemberId(1), MemberId(2)]));
        let left = channel.apply(&Event::Left(MemberId(2))).unwrap();
        assert_eq!(left.to_string(), "second hat den Kanal verlassen");
        assert_eq!(names(&channel), ["first"]);
        assert_eq!(channel.speaking(), [MemberId(1)]);

        assert_eq!(channel.apply(&Event::Left(MemberId(7))), None);
    }

    #[test]
    fn a_change_shows_on_the_member_quietly() {
        let mut channel = Channel::default();
        channel.apply(&entered(1, &[member(1, "first"), member(2, "second")]));
        let muted = Member { muted: true, ..member(2, "second") };
        assert_eq!(channel.apply(&Event::Changed(muted.clone())), None);
        assert_eq!(channel.members(), [member(1, "first"), muted]);
    }

    #[test]
    fn joining_again_keeps_the_members_until_the_new_welcome() {
        let mut channel = Channel::default();
        channel.apply(&entered(1, &[member(1, "first"), member(2, "second")]));
        channel.apply(&Event::Speaking(vec![MemberId(2)]));

        let lost = channel.apply(&Event::Reconnecting).unwrap();
        assert_eq!(lost.to_string(), "Verbindung verloren");
        assert_eq!(channel.state(), State::Reconnecting);
        assert_eq!(names(&channel), ["first", "second"]);
        assert_eq!(channel.speaking(), []);

        let back = channel.apply(&entered(4, &[member(3, "second"), member(4, "first")])).unwrap();
        assert_eq!(back.to_string(), "Wieder verbunden");
        assert_eq!(channel.you(), Some(MemberId(4)));
        assert_eq!(names(&channel), ["second", "first"]);
    }

    #[test]
    fn a_silent_server_shows_reconnecting_once_until_it_is_heard_again() {
        let mut channel = Channel::default();
        channel.apply(&entered(1, &[member(1, "first")]));
        assert_eq!(channel.apply(&Event::Reconnecting), Some(Announcement::Reconnecting));
        assert_eq!(channel.apply(&Event::Reconnecting), None);
        assert_eq!(channel.state(), State::Reconnecting);

        assert_eq!(channel.apply(&Event::Heard), Some(Announcement::Reconnected));
        assert_eq!(channel.state(), State::Connected);
        assert_eq!(names(&channel), ["first"]);
    }

    #[test]
    fn a_call_that_ends_leaves_the_channel_empty_but_tells_which_call_it_was() {
        let lost = || Lost::Unrecovered { connection: Error::TimedOut, rejoin: Error::TimedOut };
        for end in [Event::Ended, Event::Lost(lost())] {
            let mut channel = Channel::default();
            assert_eq!(channel.call(), 0);
            channel.apply(&entered(1, &[member(1, "first"), member(2, "second")]));
            assert_eq!(channel.apply(&end), None);
            assert_eq!(channel.call(), 1);
            assert_eq!(channel.state(), State::Out);
            assert_eq!(channel.you(), None);
            assert_eq!(names(&channel), [] as [&str; 0]);
        }
    }

    #[test]
    fn only_the_watch_tells_who_is_present() {
        let lost = || Lost::Unrecovered { connection: Error::TimedOut, rejoin: Error::TimedOut };
        for end in [Event::Ended, Event::Lost(lost())] {
            let mut channel = Channel::default();
            channel.apply(&Event::Present(vec![member(1, "first")]));
            channel.apply(&entered(2, &[member(1, "first"), member(2, "second")]));
            channel.apply(&Event::Present(vec![member(1, "first"), member(2, "second")]));
            channel.apply(&end);
            assert_eq!(channel.present(), [member(1, "first"), member(2, "second")]);
        }
    }
}
