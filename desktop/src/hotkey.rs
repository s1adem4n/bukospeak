use futures::channel::mpsc::UnboundedSender;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::{Action, send};

pub const KEYS: &str = if cfg!(target_os = "macos") { "⌃⇧M" } else { "Strg ⇧ M" };

pub struct Hotkey {
    _manager: GlobalHotKeyManager,
}

impl Hotkey {
    pub fn new(actions: UnboundedSender<Action>) -> Option<Self> {
        // On Windows and macOS the presses arrive through the loop of the thread that registers the key.
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
