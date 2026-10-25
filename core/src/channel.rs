//! The channel as this member sees it, kept from the engine's events: who is in it, who speaks,
//! and whether the call is up; and what to tell the user aloud about each change.

use std::fmt;

use bukospeak_proto::MemberId;
use bukospeak_proto::control::Member;

use crate::session::Error;

/// What happens in the calls, in order; the events between an [`Entered`](Event::Entered) and
/// the next [`Ended`](Event::Ended) or [`Lost`](Event::Lost) are one call's.
#[derive(Debug)]
pub(crate) enum Event {
    /// A call started, or joined again, with everyone in the channel, this member included.
    Entered {
        you: MemberId,
        members: Vec<Member>,
    },
    Joined(Member),
    Left(MemberId),
    /// Who speaks now, in id order, this member among them; sent when it changes.
    Speaking(Vec<MemberId>),
    /// The connection was lost, and the engine joins again.
    Reconnecting,
    /// The call ended with a leave.
    Ended,
    /// The call ended without a leave.
    Lost(Lost),
}

/// Why a call ended without a leave: its connection was lost, and joining again failed.
#[derive(Debug, thiserror::Error)]
#[error("{connection}; joining again failed: {rejoin}")]
pub struct Lost {
    pub connection: Error,
    pub rejoin: Error,
}

/// Whether this member is in the channel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Out,
    Connected,
    /// The connection was lost, and the engine joins again.
    Reconnecting,
}

/// The channel as this member sees it.
#[derive(Clone, Debug, Default)]
pub struct Channel {
    state: State,
    you: Option<MemberId>,
    members: Vec<Member>,
    speaking: Vec<MemberId>,
}

impl Channel {
    pub fn state(&self) -> State {
        self.state
    }

    /// This member's id, while in the channel.
    pub fn you(&self) -> Option<MemberId> {
        self.you
    }

    /// Everyone in the channel, this member included, in the order they came; kept while
    /// joining again, empty when out.
    pub fn members(&self) -> &[Member] {
        &self.members
    }

    /// Who speaks now, in id order, this member among them.
    pub fn speaking(&self) -> &[MemberId] {
        &self.speaking
    }

    /// Follows the event, and tells what to say aloud about it, if anything.
    pub(crate) fn apply(&mut self, event: &Event) -> Option<Announcement> {
        match event {
            Event::Entered { you, members } => {
                let rejoined = self.state == State::Reconnecting;
                *self = Self { state: State::Connected, you: Some(*you), members: members.clone(), speaking: Vec::new() };
                rejoined.then_some(Announcement::Reconnected)
            }
            Event::Joined(member) => {
                self.members.push(member.clone());
                Some(Announcement::Joined(member.name.clone()))
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
                self.state = State::Reconnecting;
                self.speaking.clear();
                Some(Announcement::Reconnecting)
            }
            Event::Ended | Event::Lost(_) => {
                *self = Self::default();
                None
            }
        }
    }
}

/// What to tell the user aloud about a change, as TeamSpeak does; its text is German.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Announcement {
    /// Someone else joined, by name.
    Joined(String),
    /// Someone else left, by name.
    Left(String),
    /// The connection was lost, and the engine joins again.
    Reconnecting,
    /// The engine joined again after the connection was lost.
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
        Member { id: MemberId(id), name: name.into() }
    }

    fn entered(you: u16, members: &[Member]) -> Event {
        Event::Entered { you: MemberId(you), members: members.to_vec() }
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
    fn a_call_that_ends_leaves_the_channel_empty() {
        let lost = || Lost { connection: Error::TimedOut, rejoin: Error::TimedOut };
        for end in [Event::Ended, Event::Lost(lost())] {
            let mut channel = Channel::default();
            channel.apply(&entered(1, &[member(1, "first"), member(2, "second")]));
            assert_eq!(channel.apply(&end), None);
            assert_eq!(channel.state(), State::Out);
            assert_eq!(channel.you(), None);
            assert_eq!(names(&channel), [] as [&str; 0]);
        }
    }
}
