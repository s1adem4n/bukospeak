use crate::audio::Direction;
use crate::call::{Failure, Notice};
use bukospeak_core::proto::control::Rejection;

use crate::ui::Trouble;

pub fn failure(failure: &Failure) -> &'static str {
    match failure {
        Failure::Rejected(Rejection::Stranger) => "Nicht im Tailscale-Netz des Servers",
        Failure::Rejected(Rejection::Unidentified) => "Server erreicht Tailscale nicht",
        Failure::Rejected(Rejection::Full) => "Kanal ist voll",
        Failure::Rejected(Rejection::Version) => "Server-Version passt nicht",
        Failure::Rejected(Rejection::Unavailable) => "Download nicht verfügbar",
        Failure::NotFound => "Server nicht gefunden",
        Failure::NoAnswer => "Server antwortet nicht",
        Failure::Unreachable => "Keine Verbindung zum Server",
        Failure::Audio => "Audio nicht gestartet",
        Failure::ConnectionLost => "Verbindung verloren",
        Failure::Replaced => "Auf diesem Gerät erneut beigetreten",
    }
}

pub fn notice(notice: &Notice) -> &'static str {
    match notice {
        Notice::DeviceMissing(Direction::Input) => "Mikrofon nicht gefunden",
        Notice::DeviceMissing(Direction::Output) => "Ausgabegerät nicht gefunden",
        Notice::DeviceFailed(Direction::Input) => "Mikrofon gestört",
        Notice::DeviceFailed(Direction::Output) => "Ausgabegerät gestört",
    }
}

pub fn trouble(trouble: Trouble) -> &'static str {
    match trouble {
        Trouble::Unreadable => "Einstellungen nicht lesbar",
        Trouble::NotSaved => "Einstellungen nicht gespeichert",
        Trouble::NvidiaUnavailable => "NVIDIA-Effekte nicht verfügbar",
        Trouble::NvidiaNotInstalled => "NVIDIA-Effekte nicht geladen",
        Trouble::NvidiaFailed => "NVIDIA-Effekt nicht gestartet",
        Trouble::StartNotSet => "Autostart nicht eingerichtet",
    }
}
