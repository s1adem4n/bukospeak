const OPEN: &str = "Öffnen";
const JOIN: &str = "Beitreten";
const LEAVE: &str = "Verlassen";
const MUTE: &str = "Stummschalten";
const QUIT: &str = "Beenden";

const ICON: &[u8] = include_bytes!("../assets/tray.rgba");
const ICON_SIZE: u32 = 64;

const DOT: f32 = 11.;
const DOT_GAP: f32 = 4.;
const GREEN: [u8; 3] = [0x3d, 0xd6, 0x8c];
const RED: [u8; 3] = [0xe5, 0x48, 0x4d];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    Out,
    In,
    Muted,
}

impl Standing {
    fn call_label(self) -> &'static str {
        match self {
            Self::Out => JOIN,
            Self::In | Self::Muted => LEAVE,
        }
    }
}

fn icon(standing: Standing) -> Vec<u8> {
    let color = match standing {
        Standing::Out => return ICON.to_vec(),
        Standing::In => GREEN,
        Standing::Muted => RED,
    };
    let center = ICON_SIZE as f32 - DOT;
    let mut pixels = ICON.to_vec();
    for (index, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = ((index as u32 % ICON_SIZE) as f32 + 0.5, (index as u32 / ICON_SIZE) as f32 + 0.5);
        let distance = ((x - center).powi(2) + (y - center).powi(2)).sqrt();
        let antialiased_coverage = |radius: f32| (radius + 0.5 - distance).clamp(0., 1.);
        let inside = antialiased_coverage(DOT);
        let alpha = f32::from(pixel[3]) * (1. - antialiased_coverage(DOT + DOT_GAP));
        let [red, green, blue] = color;
        let dot = 255. * inside;
        let total = dot + alpha * (1. - inside);
        if total > 0. {
            for (channel, dotted) in pixel[..3].iter_mut().zip([red, green, blue]) {
                *channel = ((f32::from(dotted) * dot + f32::from(*channel) * alpha * (1. - inside)) / total).round() as u8;
            }
        }
        pixel[3] = total.round() as u8;
    }
    pixels
}

pub use platform::Tray;

#[cfg(target_os = "linux")]
mod platform {
    use futures::channel::mpsc::UnboundedSender;
    use ksni::blocking::{Handle, TrayMethods};
    use ksni::menu::{CheckmarkItem, MenuItem, StandardItem};

    use super::{ICON_SIZE, MUTE, OPEN, QUIT, Standing, icon};
    use crate::{Action, send};

    struct Item {
        standing: Standing,
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
            let pixels = icon(self.standing);
            let data = pixels.as_chunks::<4>().0.iter().flat_map(|&[red, green, blue, alpha]| [alpha, red, green, blue]).collect();
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
                StandardItem {
                    label: self.standing.call_label().into(),
                    activate: Box::new(|item: &mut Self| send(&item.actions, Action::ToggleCall)),
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
        pub fn new(actions: UnboundedSender<Action>) -> Option<Self> {
            match (Item { standing: Standing::Out, muted: false, actions }).spawn() {
                Ok(handle) => Some(Self(handle)),
                Err(error) => {
                    tracing::info!("no tray: {error}");
                    None
                }
            }
        }

        pub fn show(&self, standing: Standing, muted: bool) {
            // Fails only once the tray's service stopped, and with it the icon.
            let _ = self.0.update(|item| {
                item.standing = standing;
                item.muted = muted;
            });
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod platform {
    use std::cell::Cell;

    use futures::channel::mpsc::UnboundedSender;
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use super::{ICON_SIZE, MUTE, OPEN, QUIT, Standing, icon};
    use crate::{Action, send};

    fn picture(standing: Standing) -> Icon {
        Icon::from_rgba(icon(standing), ICON_SIZE, ICON_SIZE).expect("the icon is as large as it says")
    }

    pub struct Tray {
        call: MenuItem,
        mute: CheckMenuItem,
        icon: TrayIcon,
        shown: Cell<Standing>,
    }

    impl Tray {
        pub fn new(actions: UnboundedSender<Action>) -> Option<Self> {
            let standing = Standing::Out;
            let (open, call, mute, quit) = (
                MenuItem::new(OPEN, true, None),
                MenuItem::new(standing.call_label(), true, None),
                CheckMenuItem::new(MUTE, true, false, None),
                MenuItem::new(QUIT, true, None),
            );
            let menu = Menu::new();
            if let Err(error) = menu.append_items(&[&open, &call, &mute, &PredefinedMenuItem::separator(), &quit]) {
                tracing::warn!("no tray: {error}");
                return None;
            }
            // As each system has it: a click shows the menu on macOS and opens the window on
            // Windows, whose menu is on the right button.
            let built = TrayIconBuilder::new()
                .with_icon(picture(standing))
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
            let ids = [open.id().clone(), call.id().clone(), mute.id().clone(), quit.id().clone()];
            let menu_actions = actions.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let Some(index) = ids.iter().position(|id| *id == event.id) else { return };
                send(&menu_actions, [Action::Open, Action::ToggleCall, Action::ToggleMute, Action::Quit][index]);
            }));
            if cfg!(target_os = "windows") {
                TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        send(&actions, Action::Open);
                    }
                }));
            }
            Some(Self { call, mute, icon, shown: Cell::new(standing) })
        }

        pub fn show(&self, standing: Standing, muted: bool) {
            self.mute.set_checked(muted);
            if self.shown.replace(standing) != standing {
                self.call.set_text(standing.call_label());
                if let Err(error) = self.icon.set_icon(Some(picture(standing))) {
                    tracing::warn!("the tray's icon: {error}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
        let at = ((y * ICON_SIZE + x) * 4) as usize;
        pixels[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn out_of_a_call_the_icon_is_the_microphone_alone() {
        assert_eq!(icon(Standing::Out), ICON);
    }

    #[test]
    fn in_a_call_a_dot_of_its_color_sits_in_the_corner_and_the_rest_stays() {
        let center = ICON_SIZE - DOT as u32;
        for (standing, color) in [(Standing::In, GREEN), (Standing::Muted, RED)] {
            let pixels = icon(standing);
            let [red, green, blue, alpha] = pixel(&pixels, center, center);
            assert_eq!(([red, green, blue], alpha), (color, 255));
            assert_eq!(pixel(&pixels, 0, 0), pixel(ICON, 0, 0));
            assert_eq!(pixel(&pixels, ICON_SIZE / 2, ICON_SIZE / 4), pixel(ICON, ICON_SIZE / 2, ICON_SIZE / 4));
        }
    }
}
