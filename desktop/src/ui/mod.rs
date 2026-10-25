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

use bukospeak_audio_desktop::Direction;
use bukospeak_desktop::call::{Call, Failure, Happening, Phase};
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

pub use crate::ui::glyph::Assets;
use crate::ui::glyph::Glyph;

/// The microphone of the app's icon, cut out, as the iOS app shows it.
const MARK: &[u8] = include_bytes!("../../assets/mark.png");

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
    /// The keyring couldn't be read: the room key is to be given anew.
    RoomKeyUnreadable,
    RoomKeyNotKept,
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
    Settings(Form, settings::Audio),
}

pub struct Main {
    call: Call,
    settings: Settings,
    settings_path: PathBuf,
    /// The room key; None until the keyring answered, or one is given before.
    room_key: Option<String>,
    screen: Screen,
    trouble: Option<Trouble>,
    mark: Arc<Image>,
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
        settings_path: PathBuf,
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
            let read = cx.background_spawn(async { stored::room_key() }).await;
            // The window may be gone before the keyring answered.
            let _ = this.update_in(cx, |main, window, cx| main.read_room_key(read, window, cx));
        });
        let appearance = cx.observe_window_appearance(window, |_, window, cx| Theme::sync_system_appearance(Some(window), cx));
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut main = Self {
            call,
            settings,
            settings_path,
            room_key: None,
            screen: Screen::Channel,
            trouble,
            mark: Arc::new(Image::from_bytes(ImageFormat::Png, MARK.to_vec())),
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

    /// Takes in the room key the keyring kept, unless one was given meanwhile, into an open
    /// form's field too while it is empty; asks for it if there is none.
    fn read_room_key(&mut self, read: Result<Option<String>, stored::Error>, window: &mut Window, cx: &mut Context<Self>) {
        let key = read.unwrap_or_else(|error| {
            tracing::warn!("the room key isn't readable: {error}");
            self.trouble = Some(Trouble::RoomKeyUnreadable);
            None
        });
        if self.room_key.is_some() {
            return;
        }
        let key = key.unwrap_or_default();
        if let Screen::Welcome(form, _) | Screen::Settings(form, _) = &self.screen
            && form.value(Field::RoomKey, cx).is_empty()
        {
            form.room_key.update(cx, |input, cx| input.set_value(key.clone(), window, cx));
        }
        self.room_key = Some(key);
        if matches!(self.screen, Screen::Channel)
            && let Some(step) = self.missing()
        {
            self.welcome(step, window, cx);
        }
        self.watch();
        cx.notify();
    }

    fn room_key(&self) -> &str {
        self.room_key.as_deref().unwrap_or_default()
    }

    /// The welcome's step that asks for what is missing to join, if anything is.
    fn missing(&self) -> Option<Step> {
        if self.settings.name.trim().is_empty() {
            Some(Step::Name)
        } else {
            (!self.settings.is_complete(self.room_key())).then_some(Step::Place)
        }
    }

    fn join(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.missing() {
            Some(step) => self.welcome(step, window, cx),
            None => {
                self.trouble = None;
                self.call.join(self.settings.join(self.room_key()));
            }
        }
        cx.notify();
    }

    fn trust(&mut self, cx: &mut Context<Self>) {
        self.call.trust(self.settings.join(self.room_key()));
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

    /// Watches who is in the channel while it is shown from outside.
    fn watch(&mut self) {
        let outside = matches!(self.call.phase(), Phase::Idle | Phase::Failed(_)) && matches!(self.screen, Screen::Channel);
        let channel = (outside && self.settings.is_complete(self.room_key()))
            .then(|| (self.settings.server.trim().to_owned(), self.room_key().to_owned()));
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
        let room_key = input(self.room_key(), "Raumschlüssel", true);
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
            (Screen::Settings(..), _) => self.close_settings(window, cx),
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
        self.keep(window, cx);
        self.join(window, cx);
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.call.dismiss();
        self.screen = Screen::Settings(self.form(window, cx), self.audio(window, cx));
        window.focus(&self.focus, cx);
        self.watch();
        cx.notify();
    }

    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.keep(window, cx);
        self.watch();
        cx.notify();
    }

    /// Takes in what the form says, keeps it, and goes back to the channel, which takes the
    /// focus from the form's fields.
    fn keep(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Screen::Welcome(form, _) | Screen::Settings(form, _)) = std::mem::replace(&mut self.screen, Screen::Channel) else {
            return;
        };
        window.focus(&self.focus, cx);
        let settings = Settings { name: form.value(Field::Name, cx), server: form.value(Field::Server, cx), ..self.settings.clone() };
        if settings != self.settings {
            if let Err(error) = settings.save(&self.settings_path) {
                tracing::warn!("{error}");
                self.trouble = Some(Trouble::NotSaved);
            }
            self.settings = settings;
        }
        let room_key = form.value(Field::RoomKey, cx);
        if room_key != self.room_key() {
            self.room_key = Some(room_key.clone());
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

    /// Changes a setting the call takes at once, and keeps it.
    fn change(&mut self, edit: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        edit(&mut self.settings);
        if let Err(error) = self.settings.save(&self.settings_path) {
            tracing::warn!("{error}");
            self.trouble = Some(Trouble::NotSaved);
        }
        cx.notify();
    }

    fn set_device(&mut self, direction: Direction, id: Option<String>, cx: &mut Context<Self>) {
        self.change(|settings| settings.set_device_id(direction, id), cx);
        self.call.set_device(direction, self.settings.device(direction));
    }

    fn set_noise_suppression(&mut self, on: bool, cx: &mut Context<Self>) {
        self.change(|settings| settings.noise_suppression = on, cx);
        self.call.set_noise_suppression(on);
    }

    fn set_echo_cancellation(&mut self, on: bool, cx: &mut Context<Self>) {
        self.change(|settings| settings.echo_cancellation = on, cx);
        self.call.set_echo_cancellation(on);
    }

    fn back(&mut self, _: &Back, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.call.phase(), Phase::Failed(Failure::CertificateChanged { .. })) {
            self.dismiss(cx);
            return;
        }
        match &self.screen {
            Screen::Settings(..) => self.close_settings(window, cx),
            Screen::Welcome(_, Step::Place) => self.previous(window, cx),
            Screen::Welcome(_, Step::Name) | Screen::Channel => cx.propagate(),
        }
    }

    /// From the place back to the name.
    fn previous(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Welcome(form, step @ Step::Place) = &mut self.screen else { return };
        *step = Step::Name;
        form.focus_empty(Step::Name.fields(), window, cx);
        cx.notify();
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> TitleBar {
        let theme = cx.theme();
        let title = match &self.screen {
            Screen::Channel => Some(div().font_weight(FontWeight::SEMIBOLD).child("BukoSpeak").into_any_element()),
            // The way back and the title in one: the arrow lines up on the gutter.
            Screen::Settings(..) => Some(
                Button::new("back")
                    .custom(
                        ButtonCustomVariant::new(cx)
                            .foreground(theme.foreground)
                            .hover(theme.secondary_hover)
                            .active(theme.secondary_active),
                    )
                    .h(theme::CONTROL)
                    .ml(-theme::CONTROL_INSET)
                    .px(theme::CONTROL_INSET)
                    .rounded(ButtonRounded::Size(theme.radius))
                    .icon(Icon::from(Glyph::Back))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Einstellungen"))
                    .on_click(cx.listener(|main, _, window, cx| main.close_settings(window, cx)))
                    .into_any_element(),
            ),
            Screen::Welcome(..) => None,
        };
        let settings = matches!(self.screen, Screen::Channel).then(|| {
            control("settings", Glyph::Settings, "Einstellungen", Tone::Plain, cx)
                .on_click(cx.listener(|main, _, window, cx| main.open_settings(window, cx)))
        });
        TitleBar::new().h(theme::TITLE_BAR).bg(theme.background).border_0().child(
            h_flex()
                .w_full()
                .h_full()
                .pr(theme::GUTTER - theme::CONTROL_INSET)
                .justify_between()
                .child(div().children(title))
                .children(settings),
        )
    }

    /// One short line of what went wrong beside the call: its notice, else the settings'
    /// trouble.
    fn render_message(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = cx.theme();
        let message = match (self.call.notice(), self.trouble) {
            (Some(notice), _) => text::notice(notice),
            (None, Some(trouble)) => text::trouble(trouble),
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
            Screen::Settings(form, audio) => self.render_settings(form, audio, cx).into_any_element(),
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
