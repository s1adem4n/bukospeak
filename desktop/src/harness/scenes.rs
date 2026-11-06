//! The states of the window worth looking at, each with what it takes to show it.

use std::time::Duration;

use bukospeak_core::proto::control::{Member, Rejection};
use bukospeak_core::proto::{Fingerprint, MemberId};

use crate::audio::Direction;
use crate::call::{Failure, Notice, Phase, Stage};
use crate::nvidia::{Architecture, Progress};
use crate::settings::{Settings, Suppression};

/// A state of the window: the settings, the call as staged, the system as staged, and the steps
/// that lead from the window opened on it to what it shows.
pub struct Scene {
    pub name: &'static str,
    pub about: &'static str,
    pub settings: Settings,
    pub room_key: Option<&'static str>,
    pub card: Option<Architecture>,
    pub installing: Progress,
    pub stage: Stage,
    pub steps: &'static str,
}

/// The member this window is.
const YOU: MemberId = MemberId(1);

fn member(id: u16, name: &str) -> Member {
    Member { id: MemberId(id), name: name.into(), muted: false, reconnecting: false }
}

/// The usual call: this member and one other.
fn two() -> Vec<Member> {
    vec![member(1, "Nachtfalke"), member(2, "Kiwi")]
}

/// A larger channel: this member and four others, one of them muted.
fn five() -> Vec<Member> {
    vec![
        member(1, "Nachtfalke"),
        member(2, "Kiwi"),
        Member { muted: true, ..member(3, "Mondschein") },
        member(4, "Pixel"),
        member(5, "Zugvogel"),
    ]
}

fn complete() -> Settings {
    Settings { name: "Nachtfalke".into(), server: "voice.example:9988".into(), ..Settings::default() }
}

fn idle() -> Stage {
    Stage {
        phase: Phase::Idle,
        you: None,
        members: Vec::new(),
        speaking: Vec::new(),
        present: Vec::new(),
        muted: false,
        notice: None,
        up_for: None,
    }
}

fn in_call() -> Stage {
    Stage {
        phase: Phase::Connected,
        you: Some(YOU),
        members: two(),
        speaking: vec![MemberId(2)],
        up_for: Some(Duration::from_secs(47 * 60 + 12)),
        ..idle()
    }
}

fn scene(name: &'static str, about: &'static str, stage: Stage) -> Scene {
    Scene {
        name,
        about,
        settings: complete(),
        room_key: Some("room-key"),
        card: None,
        installing: Progress::Preparing { done: 0, total: 0 },
        stage,
        steps: "",
    }
}

pub fn all() -> Vec<Scene> {
    let fingerprint: Fingerprint =
        "c8:69:8f:c4:83:66:58:80:12:28:d6:ef:a0:6e:62:65:af:6c:b9:09:c3:8c:a4:3a:7d:25:9f:1d:a5:a2:95:81".parse().expect("a fingerprint");
    let other: Fingerprint =
        "10:e7:e6:ac:63:24:fd:51:77:44:f1:ca:5e:71:10:1a:72:0d:eb:c4:58:76:cd:61:92:f0:7e:3e:46:8d:80:55".parse().expect("a fingerprint");
    vec![
        Scene { settings: Settings::default(), room_key: None, ..scene("welcome", "the first start: the name", idle()) },
        Scene {
            settings: Settings { name: "Nachtfalke".into(), ..Settings::default() },
            room_key: None,
            ..scene("welcome-place", "the first start: the server and its room key", idle())
        },
        scene("lobby", "out of a call, another in the channel", Stage { present: two()[1..].to_vec(), ..idle() }),
        scene("lobby-three", "out of a call, three in the channel", Stage { present: five()[1..4].to_vec(), ..idle() }),
        scene("lobby-empty", "out of a call, nobody in the channel", idle()),
        scene(
            "joining",
            "the join under way, another in the channel",
            Stage { phase: Phase::Joining, present: two()[1..].to_vec(), ..idle() },
        ),
        scene("call", "in a call of two, the other speaking", in_call()),
        scene("call-four", "in a call of four, one speaking, one muted", Stage { members: five()[..4].to_vec(), ..in_call() }),
        scene(
            "call-five",
            "in a call of five, two speaking, one muted",
            Stage { members: five(), speaking: vec![MemberId(2), MemberId(4)], ..in_call() },
        ),
        scene("call-alone", "in a call alone", Stage { members: two()[..1].to_vec(), speaking: Vec::new(), ..in_call() }),
        scene("call-muted", "in a call, muted", Stage { muted: true, speaking: Vec::new(), ..in_call() }),
        scene("call-speaking", "in a call, speaking oneself", Stage { speaking: vec![YOU], ..in_call() }),
        scene(
            "reconnecting",
            "the connection lost, joining again",
            Stage { phase: Phase::Reconnecting, speaking: Vec::new(), ..in_call() },
        ),
        scene(
            "member-reconnecting",
            "in a call, another member reconnecting",
            Stage {
                members: two()
                    .into_iter()
                    .map(|member| if member.id == MemberId(2) { Member { reconnecting: true, ..member } } else { member })
                    .collect(),
                speaking: Vec::new(),
                ..in_call()
            },
        ),
        scene("notice", "in a call, the microphone gone", Stage { notice: Some(Notice::DeviceMissing(Direction::Input)), ..in_call() }),
        scene(
            "failed",
            "a join that found no server",
            Stage { phase: Phase::Failed(Failure::Unreachable), present: two()[1..].to_vec(), ..idle() },
        ),
        scene(
            "rejected",
            "a join with the wrong room key",
            Stage { phase: Phase::Failed(Failure::Rejected(Rejection::RoomKey)), ..idle() },
        ),
        scene(
            "certificate",
            "the server's certificate changed",
            Stage {
                phase: Phase::Failed(Failure::CertificateChanged {
                    server: "voice.example:9988".into(),
                    pinned: fingerprint,
                    presented: other,
                }),
                ..idle()
            },
        ),
        Scene { steps: "click settings", ..scene("settings", "the settings", idle()) },
        Scene {
            card: Some(Architecture::Ampere),
            settings: Settings { suppression: Suppression::NvidiaStudioVoice, ..complete() },
            installing: Progress::Fetching { done: 37, total: 100 },
            steps: "click settings",
            ..scene("settings-nvidia", "the settings, NVIDIA's Studio Voice installing", idle())
        },
        Scene {
            card: Some(Architecture::Ampere),
            steps: "click settings; click suppression",
            ..scene("settings-suppression", "the settings, the menu of noise suppressions open", idle())
        },
        Scene { steps: "click settings; click input", ..scene("settings-input", "the settings, the menu of microphones open", idle()) },
    ]
}
