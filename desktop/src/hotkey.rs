//! The key that mutes and unmutes from any app: Ctrl+Shift+M, on its press.
//!
//! Windows and macOS have such keys for every app; on Linux only X11 does, as Wayland's global
//! shortcuts go through a portal the user is asked in, which the app doesn't use yet.

use futures::channel::mpsc::UnboundedSender;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::{Action, send};

/// The registered key; it is unregistered when this drops.
pub struct Hotkey {
    _manager: GlobalHotKeyManager,
}

impl Hotkey {
    /// Registers the key, which then asks the app to toggle the mute; None if another app has it
    /// or the system has no keys for every app. Made on the app's thread, whose loop gets its
    /// presses on Windows and macOS.
    pub fn new(actions: UnboundedSender<Action>) -> Option<Self> {
        let key = HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyM);
        let manager = GlobalHotKeyManager::new().and_then(|manager| manager.register(key).map(|()| manager));
        let manager = match manager {
            Ok(manager) => manager,
            Err(error) => {
                tracing::warn!("no hotkey: {error}");
                return None;
            }
        };
        let id = key.id();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.id == id && event.state == HotKeyState::Pressed {
                send(&actions, Action::ToggleMute);
            }
        }));
        Some(Self { _manager: manager })
    }
}
