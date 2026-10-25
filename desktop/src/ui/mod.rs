//! The window: a welcome the first time, then the channel, who is in it and the bar to join,
//! mute and leave; the settings; and the dialog for a changed certificate.

mod certificate;
mod channel;
mod glyph;
mod member;
mod settings;
mod text;
pub mod theme;
mod welcome;

use std::path::PathBuf;
use std::sync::Arc;

use bukospeak_desktop::call::{Call, Devices, Failure, Happening, Join, Phase};
use bukospeak_desktop::settings::{self as stored, Settings};
use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonRounded, ButtonVariants};
use gpui_kit::component::input::{Copy, Cut, InputEvent, InputState, Paste, SelectAll};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::{ActiveTheme, Icon, Sizable, Size, Theme, TitleBar, h_flex};
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, FocusHandle, FontWeight, Image, ImageFormat, InteractiveElement, IntoElement,
    ParentElement, Render, Styled, Subscription, Task, Window, actions, div,
};

use crate::ui::glyph::Glyph;

/// The app's icon, as the iOS app has it.
const ICON: &[u8] = include_bytes!("../../assets/icon.png");

actions!(
    bukospeak,
    [
        /// Escape: closes the dialog or the settings, or goes a step back.
        Back
    ]
);

/// The key context of the window's content, which Back is bound in.
pub const CONTEXT: &str = "Main";

/// What went wrong with the settings themselves, beside the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trouble {
    /// The settings file couldn't be read: the app starts from the defaults.
    Unreadable,
    NotSaved,
    RoomKeyNotKept,
}

impl Trouble {
    fn text(self) -> &'static str {
        match self {
            Self::Unreadable => "Einstellungen nicht lesbar",
            Self::NotSaved => "Einstellungen nicht gespeichert",
            Self::RoomKeyNotKept => "Raumschlüssel nicht gespeichert",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Name,
    Server,
    RoomKey,
}

/// The fields of the welcome or the settings.
struct Form {
    name: Entity<InputState>,
    server: Entity<InputState>,
    room_key: Entity<InputState>,
    _events: Vec<Subscription>,
}

impl Form {
    fn input(&self, field: Field) -> &Entity<InputState> {
        match field {
            Field::Name => &self.name,
            Field::Server => &self.server,
            Field::RoomKey => &self.room_key,
        }
    }

    fn value(&self, field: Field, cx: &App) -> String {
        self.input(field).read(cx).value().trim().to_owned()
    }

    /// Focuses the first of `fields` still empty, else the first; true if one was empty.
    fn focus_empty(&self, fields: &[Field], window: &mut Window, cx: &mut App) -> bool {
        let empty = fields.iter().find(|field| self.value(**field, cx).is_empty());
        let field = empty.or(fields.first()).expect("there is a field to focus");
        self.input(*field).update(cx, |input, cx| input.focus(window, cx));
        empty.is_some()
    }
}

/// The welcome's steps: who one is, then where the others are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Name,
    Place,
}

impl Step {
    fn fields(self) -> &'static [Field] {
        match self {
            Self::Name => &[Field::Name],
            Self::Place => &[Field::Server, Field::RoomKey],
        }
    }
}

enum Screen {
    Channel,
    Welcome(Form, Step),
    Settings(Form),
}

pub struct Main {
    call: Call,
    settings: Settings,
    /// Where the settings are kept; None if the system has no config folder.
    settings_path: Option<PathBuf>,
    room_key: String,
    screen: Screen,
    trouble: Option<Trouble>,
    icon: Arc<Image>,
    focus: FocusHandle,
    _happenings: Task<()>,
    _room_key: Task<()>,
    _appearance: Subscription,
}

impl Main {
    pub fn new(
        call: Call,
        mut happenings: UnboundedReceiver<Happening>,
        settings: Settings,
        settings_path: Option<PathBuf>,
        trouble: Option<Trouble>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        call.set_noise_suppression(settings.noise_suppression);
        call.set_echo_cancellation(settings.echo_cancellation);
        let happenings = cx.spawn(async move |this, cx| {
            while let Some(happening) = happenings.next().await {
                let applied = this.update(cx, |main, cx| {
                    main.call.happen(happening);
                    main.watch();
                    cx.notify();
                });
                if applied.is_err() {
                    break;
                }
            }
        });
        // The keyring may ask the system, and the system the user: not on the window's thread.
        let room_key = cx.spawn_in(window, async move |this, cx| {
            let key = cx.background_spawn(async { stored::room_key() }).await;
            // The window may be gone before the keyring answered.
            let _ = this.update_in(cx, |main, window, cx| {
                match key {
                    Ok(key) => main.room_key = key.unwrap_or_default(),
                    Err(error) => {
                        tracing::warn!("the room key isn't readable: {error}");
                        main.trouble = Some(Trouble::RoomKeyNotKept);
                    }
                }
                if matches!(main.screen, Screen::Channel) && main.room_key.is_empty() {
                    main.welcome(Step::Place, window, cx);
                }
                main.watch();
                cx.notify();
            });
        });
        let appearance = cx.observe_window_appearance(window, |_, window, cx| Theme::sync_system_appearance(Some(window), cx));
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut main = Self {
            call,
            settings,
            settings_path,
            room_key: String::new(),
            screen: Screen::Channel,
            trouble,
            icon: Arc::new(Image::from_bytes(ImageFormat::Png, ICON.to_vec())),
            focus,
            _happenings: happenings,
            _room_key: room_key,
            _appearance: appearance,
        };
        if main.settings.name.trim().is_empty() {
            main.welcome(Step::Name, window, cx);
        } else if main.settings.server.trim().is_empty() {
            main.welcome(Step::Place, window, cx);
        }
        main
    }

    fn join(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_complete(&self.room_key) {
            self.trouble = None;
            self.call.join(self.request());
        } else {
            self.welcome(if self.settings.name.trim().is_empty() { Step::Name } else { Step::Place }, window, cx);
        }
        cx.notify();
    }

    fn trust(&mut self, cx: &mut Context<Self>) {
        self.call.trust(self.request());
        cx.notify();
    }

    fn leave(&mut self, cx: &mut Context<Self>) {
        self.call.leave();
        self.watch();
        cx.notify();
    }

    fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        self.call.set_muted(!self.call.is_muted());
        cx.notify();
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.call.dismiss();
        self.watch();
        cx.notify();
    }

    /// The join the settings make.
    fn request(&self) -> Join {
        let device = |id: &Option<String>| id.as_deref().and_then(|id| id.parse().ok());
        Join {
            server: self.settings.server.trim().to_owned(),
            name: self.settings.name.trim().to_owned(),
            room_key: self.room_key.clone(),
            devices: Devices { input: device(&self.settings.input), output: device(&self.settings.output) },
        }
    }

    /// Watches who is in the channel while it is shown from outside.
    fn watch(&mut self) {
        let outside = matches!(self.call.phase(), Phase::Idle | Phase::Failed(_)) && matches!(self.screen, Screen::Channel);
        let channel =
            (outside && self.settings.is_complete(&self.room_key)).then(|| (self.settings.server.trim().to_owned(), self.room_key.clone()));
        self.call.watch(channel);
    }

    /// Fields holding what is set now; Enter goes on from each as the screen they are on says.
    fn form(&self, window: &mut Window, cx: &mut Context<Self>) -> Form {
        let mut input = |value: &str, placeholder: &'static str, masked: bool| {
            let value = value.to_owned();
            cx.new(|cx| InputState::new(window, cx).default_value(value).placeholder(placeholder).masked(masked))
        };
        let name = input(&self.settings.name, "Name", false);
        let server = input(&self.settings.server, "server.example:9988", false);
        let room_key = input(&self.room_key, "Raumschlüssel", true);
        let events = [(Field::Name, &name), (Field::Server, &server), (Field::RoomKey, &room_key)]
            .into_iter()
            .map(|(field, input)| {
                cx.subscribe_in(input, window, move |main, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => main.enter(field, window, cx),
                    // The welcome's buttons and avatar follow what is typed.
                    InputEvent::Change => cx.notify(),
                    _ => {}
                })
            })
            .collect();
        Form { name, server, room_key, _events: events }
    }

    fn enter(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        match (&self.screen, field) {
            (Screen::Settings(_), _) => self.close_settings(window, cx),
            (Screen::Welcome(_, Step::Name), _) => self.next(window, cx),
            (Screen::Welcome(form, Step::Place), Field::Server) if form.value(Field::RoomKey, cx).is_empty() => {
                form.focus_empty(&[Field::RoomKey], window, cx);
            }
            (Screen::Welcome(_, Step::Place), _) => self.finish(window, cx),
            (Screen::Channel, _) => {}
        }
    }

    fn welcome(&mut self, step: Step, window: &mut Window, cx: &mut Context<Self>) {
        let form = self.form(window, cx);
        form.focus_empty(step.fields(), window, cx);
        self.screen = Screen::Welcome(form, step);
        self.watch();
        cx.notify();
    }

    /// From the name on to the place, once there is a name.
    fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Welcome(form, step @ Step::Name) = &mut self.screen else { return };
        if form.value(Field::Name, cx).is_empty() {
            return;
        }
        *step = Step::Place;
        form.focus_empty(Step::Place.fields(), window, cx);
        cx.notify();
    }

    /// Keeps what the welcome was told and joins, once all of it is there.
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Welcome(form, _) = &self.screen else { return };
        if form.focus_empty(&[Field::Name, Field::Server, Field::RoomKey], window, cx) {
            return;
        }
        self.keep(cx);
        self.join(window, cx);
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.call.dismiss();
        self.screen = Screen::Settings(self.form(window, cx));
        window.focus(&self.focus, cx);
        self.watch();
        cx.notify();
    }

    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.keep(cx);
        window.focus(&self.focus, cx);
        self.watch();
        cx.notify();
    }

    /// Takes in what the form says, keeps it, and goes back to the channel.
    fn keep(&mut self, cx: &mut Context<Self>) {
        let (Screen::Welcome(form, _) | Screen::Settings(form)) = std::mem::replace(&mut self.screen, Screen::Channel) else {
            return;
        };
        let settings = Settings { name: form.value(Field::Name, cx), server: form.value(Field::Server, cx), ..self.settings.clone() };
        if settings != self.settings {
            if let Some(path) = &self.settings_path
                && let Err(error) = settings.save(path)
            {
                tracing::warn!("{error}");
                self.trouble = Some(Trouble::NotSaved);
            }
            self.settings = settings;
        }
        let room_key = form.value(Field::RoomKey, cx);
        if room_key != self.room_key {
            self.room_key.clone_from(&room_key);
            cx.spawn(async move |this, cx| {
                let kept = cx.background_spawn(async move { stored::set_room_key(&room_key) }).await;
                if let Err(error) = kept {
                    tracing::warn!("the room key isn't kept: {error}");
                    // The window may be gone before the keyring answered.
                    let _ = this.update(cx, |main, cx| {
                        main.trouble = Some(Trouble::RoomKeyNotKept);
                        cx.notify();
                    });
                }
            })
            .detach();
        }
    }

    fn back(&mut self, _: &Back, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.call.phase(), Phase::Failed(Failure::CertificateChanged { .. })) {
            self.dismiss(cx);
            return;
        }
        match &mut self.screen {
            Screen::Settings(_) => self.close_settings(window, cx),
            Screen::Welcome(form, step @ Step::Place) => {
                *step = Step::Name;
                form.focus_empty(Step::Name.fields(), window, cx);
                cx.notify();
            }
            Screen::Welcome(_, Step::Name) | Screen::Channel => cx.propagate(),
        }
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> TitleBar {
        let theme = cx.theme();
        let title = match &self.screen {
            Screen::Channel => Some(div().pl_1().font_weight(FontWeight::SEMIBOLD).child("BukoSpeak").into_any_element()),
            Screen::Settings(_) => Some(
                h_flex()
                    .gap_1()
                    .child(
                        control("back", Glyph::Back, "Zurück", Tone::Plain, cx)
                            .on_click(cx.listener(|main, _, window, cx| main.close_settings(window, cx))),
                    )
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Einstellungen"))
                    .into_any_element(),
            ),
            Screen::Welcome(..) => None,
        };
        let settings = matches!(self.screen, Screen::Channel).then(|| {
            control("settings", Glyph::Settings, "Einstellungen", Tone::Plain, cx)
                .on_click(cx.listener(|main, _, window, cx| main.open_settings(window, cx)))
        });
        TitleBar::new()
            .h(theme::TITLE_BAR)
            .bg(theme.background)
            .border_0()
            .child(h_flex().w_full().h_full().pr_1p5().justify_between().child(div().children(title)).children(settings))
    }

    /// One short line of what went wrong beside the call: its notice, else the settings'
    /// trouble.
    fn render_message(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = cx.theme();
        let message = match (self.call.notice(), self.trouble) {
            (Some(notice), _) => text::notice(notice),
            (None, Some(trouble)) => trouble.text(),
            (None, None) => return None,
        };
        Some(
            h_flex()
                .gap_1p5()
                .text_size(theme::TEXT_SM)
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.warning)
                .child(Icon::from(Glyph::Warning).size(theme::MARK_ICON))
                .child(message)
                .into_any_element(),
        )
    }
}

impl Render for Main {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match &self.screen {
            Screen::Channel => self.render_channel(cx).into_any_element(),
            Screen::Welcome(form, step) => self.render_welcome(form, *step, cx).into_any_element(),
            Screen::Settings(form) => self.render_settings(form, cx).into_any_element(),
        };
        let certificate = match self.call.phase() {
            Phase::Failed(Failure::CertificateChanged { pinned, presented, .. }) => Some(self.render_certificate(*pinned, *presented, cx)),
            _ => None,
        };
        let theme = cx.theme();
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme.background)
            .text_color(theme.foreground)
            .text_size(theme::TEXT)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::back))
            .child(self.render_title_bar(cx))
            .child(content)
            .children(certificate)
    }
}

/// How a small square button looks.
#[derive(Clone, Copy)]
enum Tone {
    /// An icon, a surface on hover.
    Plain,
    /// Red on a red tint: what is off now and wants attention.
    Off,
    /// Solid red: what ends something.
    End,
}

/// A small square icon button with its tooltip.
fn control(id: &'static str, glyph: Glyph, tooltip: &'static str, tone: Tone, cx: &App) -> Button {
    let theme = cx.theme();
    let button = Button::new(id);
    let custom = ButtonCustomVariant::new(cx);
    let button = match tone {
        Tone::Plain => button.custom(custom.foreground(theme.muted_foreground).hover(theme.secondary_hover).active(theme.secondary_active)),
        // A custom button draws its color as a tint.
        Tone::Off => button.custom(
            custom.color(theme.danger).foreground(theme.danger).hover(theme.danger.opacity(0.24)).active(theme.danger.opacity(0.32)),
        ),
        Tone::End => button.danger(),
    };
    button
        // The button draws its icon at three quarters of its size, so it takes the icon's size
        // and is then made as large as a control.
        .with_size(Size::Size(theme::ICON / 0.75))
        .size(theme::CONTROL)
        .rounded(ButtonRounded::Size(theme.radius))
        .icon(Icon::from(glyph))
        .tooltip(tooltip)
}

/// The button of what to do next.
fn primary(id: &'static str, label: &'static str) -> Button {
    Button::new(id).primary().large().h(theme::FIELD).rounded(ButtonRounded::Medium).label(label)
}

/// A text field's menu, in German: gpui-component's own is English.
fn edit_menu(menu: NativeMenu, _: &mut Window, _: &mut App) -> NativeMenu {
    menu.menu("Ausschneiden", Box::new(Cut))
        .menu("Kopieren", Box::new(Copy))
        .menu("Einfügen", Box::new(Paste))
        .separator()
        .menu("Alles auswählen", Box::new(SelectAll))
}
