//! The icon in the system's tray: a click opens the window, its menu opens it, mutes and quits.
//! While it is there, closing the window keeps the app and the call going.
//!
//! Linux shows it through the StatusNotifierItem interface over D-Bus, which its desktops' trays
//! read; Windows and macOS through their own tray, which `tray-icon` draws.

use futures::channel::mpsc::UnboundedSender;

/// What the tray asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Open,
    ToggleMute,
    Quit,
}

const OPEN: &str = "Öffnen";
const MUTE: &str = "Stummschalten";
const QUIT: &str = "Beenden";

/// The tray's icon, the microphone of the app's icon, 64 pixels square, as RGBA.
const ICON: &[u8] = include_bytes!("../assets/tray.rgba");
const ICON_SIZE: u32 = 64;

/// Sends an action to the app; one sent while the app quits has no one left to do it.
fn send(actions: &UnboundedSender<Action>, action: Action) {
    // A closed channel means the app is quitting, which nothing in the tray changes.
    let _ = actions.unbounded_send(action);
}

pub use platform::Tray;

#[cfg(target_os = "linux")]
mod platform {
    use futures::channel::mpsc::UnboundedSender;
    use ksni::blocking::{Handle, TrayMethods};
    use ksni::menu::{CheckmarkItem, MenuItem, StandardItem};

    use super::{Action, ICON, ICON_SIZE, MUTE, OPEN, QUIT, send};

    struct Item {
        muted: bool,
        actions: UnboundedSender<Action>,
    }

    impl ksni::Tray for Item {
        fn id(&self) -> String {
            "bukospeak".into()
        }

        fn title(&self) -> String {
            "BukoSpeak".into()
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            // StatusNotifierItem wants ARGB in network byte order.
            let data = ICON.as_chunks::<4>().0.iter().flat_map(|&[red, green, blue, alpha]| [alpha, red, green, blue]).collect();
            let size = i32::try_from(ICON_SIZE).expect("the icon is small");
            vec![ksni::Icon { width: size, height: size, data }]
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            send(&self.actions, Action::Open);
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            vec![
                StandardItem {
                    label: OPEN.into(),
                    activate: Box::new(|item: &mut Self| send(&item.actions, Action::Open)),
                    ..Default::default()
                }
                .into(),
                CheckmarkItem {
                    label: MUTE.into(),
                    checked: self.muted,
                    activate: Box::new(|item: &mut Self| send(&item.actions, Action::ToggleMute)),
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
                StandardItem {
                    label: QUIT.into(),
                    activate: Box::new(|item: &mut Self| send(&item.actions, Action::Quit)),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }

    pub struct Tray(Handle<Item>);

    impl Tray {
        /// The tray's icon, if the desktop has a tray to show it in.
        pub fn new(muted: bool, actions: UnboundedSender<Action>) -> Option<Self> {
            match (Item { muted, actions }).spawn() {
                Ok(handle) => Some(Self(handle)),
                Err(error) => {
                    tracing::info!("no tray: {error}");
                    None
                }
            }
        }

        pub fn set_muted(&self, muted: bool) {
            // None once the tray's service stopped, when there is no icon left to change.
            let _ = self.0.update(|item| item.muted = muted);
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod platform {
    use futures::channel::mpsc::UnboundedSender;
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use super::{Action, ICON, ICON_SIZE, MUTE, OPEN, QUIT, send};

    pub struct Tray {
        mute: CheckMenuItem,
        _icon: TrayIcon,
    }

    impl Tray {
        /// The tray's icon; None if the system refused it.
        pub fn new(muted: bool, actions: UnboundedSender<Action>) -> Option<Self> {
            let (open, mute, quit) =
                (MenuItem::new(OPEN, true, None), CheckMenuItem::new(MUTE, true, muted, None), MenuItem::new(QUIT, true, None));
            let menu = Menu::new();
            if let Err(error) = menu.append_items(&[&open, &mute, &PredefinedMenuItem::separator(), &quit]) {
                tracing::warn!("no tray: {error}");
                return None;
            }
            let picture = Icon::from_rgba(ICON.to_vec(), ICON_SIZE, ICON_SIZE).expect("the icon is as large as it says");
            // A click shows the menu on macOS, as there; on Windows it opens the window, and the
            // menu is on the right button, as there.
            let built = TrayIconBuilder::new()
                .with_icon(picture)
                .with_tooltip("BukoSpeak")
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(cfg!(target_os = "macos"))
                .build();
            let icon = match built {
                Ok(icon) => icon,
                Err(error) => {
                    tracing::warn!("no tray: {error}");
                    return None;
                }
            };
            let ids = (open.id().clone(), mute.id().clone(), quit.id().clone());
            let menu_actions = actions.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let action = match &event.id {
                    id if *id == ids.0 => Action::Open,
                    id if *id == ids.1 => Action::ToggleMute,
                    id if *id == ids.2 => Action::Quit,
                    _ => return,
                };
                send(&menu_actions, action);
            }));
            if cfg!(target_os = "windows") {
                TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        send(&actions, Action::Open);
                    }
                }));
            }
            Some(Self { mute, _icon: icon })
        }

        pub fn set_muted(&self, muted: bool) {
            self.mute.set_checked(muted);
        }
    }
}
