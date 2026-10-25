//! What the app tells the user, in German, of what went wrong.

use bukospeak_audio_desktop::Direction;
use bukospeak_core::proto::control::Rejection;
use bukospeak_desktop::call::{Failure, Notice};

use crate::ui::Trouble;

pub fn failure(failure: &Failure) -> &'static str {
    match failure {
        Failure::CertificateChanged { .. } => "Zertifikat geändert",
        Failure::Rejected(Rejection::RoomKey) => "Falscher Raumschlüssel",
        Failure::Rejected(Rejection::Name) => "Name nicht erlaubt",
        Failure::Rejected(Rejection::Full) => "Kanal ist voll",
        Failure::Rejected(Rejection::Version) => "Server-Version passt nicht",
        Failure::NotFound => "Server nicht gefunden",
        Failure::NoAnswer => "Server antwortet nicht",
        Failure::Unreachable => "Keine Verbindung zum Server",
        Failure::KnownServers => "Bekannte Server nicht lesbar",
        Failure::Audio => "Audio nicht gestartet",
        Failure::ConnectionLost => "Verbindung verloren",
    }
}

pub fn notice(notice: &Notice) -> &'static str {
    match notice {
        Notice::DeviceMissing(Direction::Input) => "Mikrofon nicht gefunden",
        Notice::DeviceMissing(Direction::Output) => "Ausgabegerät nicht gefunden",
        Notice::DeviceFailed(Direction::Input) => "Mikrofon gestört",
        Notice::DeviceFailed(Direction::Output) => "Ausgabegerät gestört",
        Notice::PinNotKept => "Zertifikat nicht gespeichert",
    }
}

pub fn trouble(trouble: Trouble) -> &'static str {
    match trouble {
        Trouble::Unreadable => "Einstellungen nicht lesbar",
        Trouble::NotSaved => "Einstellungen nicht gespeichert",
        Trouble::RoomKeyUnreadable => "Raumschlüssel nicht lesbar",
        Trouble::RoomKeyNotKept => "Raumschlüssel nicht gespeichert",
    }
}
