//! The window: a welcome the first time, then the channel, who is in it and the bar to join,
//! mute and leave; the settings; and the dialog for a changed certificate.

mod certificate;
mod channel;
mod client;
mod glyph;
mod member;
mod settings;
mod text;
pub mod theme;
mod welcome;

use std::sync::Arc;

use bukospeak_desktop::call::{Failure, Phase};
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonRounded, ButtonVariants};
use gpui_kit::component::input::{Copy, Cut, InputEvent, InputState, Paste, SelectAll};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::{ActiveTheme, Icon, Sizable, Size, Theme, TitleBar, h_flex};
use gpui_kit::{
    AnyElement, App, AppContext, Bounds, Context, Entity, FocusHandle, FontWeight, Image, ImageFormat, InteractiveElement, IntoElement,
    ParentElement, Render, Styled, Subscription, Window, WindowBounds, WindowOptions, actions, div, px, size,
};

pub use crate::ui::client::Client;
use crate::ui::client::RoomKeyRead;
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

/// Opens the window, which shows the client.
pub fn open(client: Entity<Client>, cx: &mut App) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(380.), px(560.)), cx))),
        window_min_size: Some(size(px(320.), px(440.))),
        app_id: Some("bukospeak".into()),
        // The content draws the title bar, so it reaches up to the window's edge.
        ..TitleBar::window_options()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        Theme::sync_system_appearance(Some(window), cx);
        cx.new(|cx| Main::new(client, window, cx))
    })
    .expect("the window opens");
    cx.activate(true);
}

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
    client: Entity<Client>,
    screen: Screen,
    mark: Arc<Image>,
    focus: FocusHandle,
    _client: [Subscription; 2],
    _appearance: Subscription,
}

impl Main {
    pub fn new(client: Entity<Client>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observed = cx.observe(&client, |_, _, cx| cx.notify());
        let read = cx.subscribe_in(&client, window, |main, _, _: &RoomKeyRead, window, cx| main.read_room_key(window, cx));
        let appearance = cx.observe_window_appearance(window, |_, window, cx| Theme::sync_system_appearance(Some(window), cx));
        // A closed window no longer shows who is in the channel.
        cx.on_release(|main, cx| main.client.update(cx, |client, _| client.set_lobby(false))).detach();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut main = Self {
            client,
            screen: Screen::Channel,
            mark: Arc::new(Image::from_bytes(ImageFormat::Png, MARK.to_vec())),
            focus,
            _client: [observed, read],
            _appearance: appearance,
        };
        match main.missing(cx) {
            Some(step) => main.welcome(step, window, cx),
            None => main.show_channel(cx),
        }
        main
    }

    /// Takes the room key the keyring kept into an open form's field while it is empty; asks
    /// for it if there is none.
    fn read_room_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.client.read(cx).room_key().to_owned();
        match &self.screen {
            Screen::Welcome(form, _) | Screen::Settings(form, _) => {
                if form.value(Field::RoomKey, cx).is_empty() {
                    form.room_key.update(cx, |input, cx| input.set_value(key, window, cx));
                }
            }
            Screen::Channel => {
                if let Some(step) = self.missing(cx) {
                    self.welcome(step, window, cx);
                }
            }
        }
    }

    /// The welcome's step that asks for what is missing to join, if anything is; the room key
    /// only once the keyring answered.
    fn missing(&self, cx: &App) -> Option<Step> {
        let client = self.client.read(cx);
        let settings = &client.settings;
        if settings.name.trim().is_empty() {
            Some(Step::Name)
        } else if settings.server.trim().is_empty() || client.is_room_key_read() && !settings.is_complete(client.room_key()) {
            Some(Step::Place)
        } else {
            None
        }
    }

    fn join(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.missing(cx) {
            Some(step) => self.welcome(step, window, cx),
            None => self.client.update(cx, |client, cx| client.join(cx)),
        }
    }

    /// Fields holding what is set now; Enter goes on from each as the screen they are on says.
    fn form(&self, window: &mut Window, cx: &mut Context<Self>) -> Form {
        let client = self.client.read(cx);
        let values = [client.settings.name.clone(), client.settings.server.clone(), client.room_key().to_owned()];
        let [name, server, room_key] = values;
        let mut input = |value: String, placeholder: &'static str, masked: bool| {
            cx.new(|cx| InputState::new(window, cx).default_value(value).placeholder(placeholder).masked(masked))
        };
        let name = input(name, "Name", false);
        let server = input(server, "server.example:9988", false);
        let room_key = input(room_key, "Raumschlüssel", true);
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

    /// Shows a screen other than the channel, so who is in it isn't watched meanwhile.
    fn show(&mut self, screen: Screen, cx: &mut Context<Self>) {
        self.screen = screen;
        self.client.update(cx, |client, _| client.set_lobby(false));
        cx.notify();
    }

    /// Shows the channel, from outside it while not in the call; the focus goes from the
    /// fields of the screen before.
    fn show_channel(&mut self, cx: &mut Context<Self>) {
        self.screen = Screen::Channel;
        self.client.update(cx, |client, _| client.set_lobby(true));
        cx.notify();
    }

    fn welcome(&mut self, step: Step, window: &mut Window, cx: &mut Context<Self>) {
        let form = self.form(window, cx);
        form.focus_empty(step.fields(), window, cx);
        self.show(Screen::Welcome(form, step), cx);
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

    /// From the place back to the name.
    fn previous(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Screen::Welcome(form, step @ Step::Place) = &mut self.screen else { return };
        *step = Step::Name;
        form.focus_empty(Step::Name.fields(), window, cx);
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
        self.client.update(cx, |client, cx| client.dismiss(cx));
        let screen = Screen::Settings(self.form(window, cx), self.audio(window, cx));
        window.focus(&self.focus, cx);
        self.show(screen, cx);
    }

    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.keep(window, cx);
    }

    /// Keeps what the form says and goes back to the channel, which takes the focus from the
    /// form's fields.
    fn keep(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Screen::Welcome(form, _) | Screen::Settings(form, _)) = std::mem::replace(&mut self.screen, Screen::Channel) else {
            return;
        };
        window.focus(&self.focus, cx);
        let [name, server, room_key] = [Field::Name, Field::Server, Field::RoomKey].map(|field| form.value(field, cx));
        self.client.update(cx, |client, cx| client.keep(name, server, room_key, cx));
        self.show_channel(cx);
    }

    fn back(&mut self, _: &Back, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.client.read(cx).call.phase(), Phase::Failed(Failure::CertificateChanged { .. })) {
            self.client.update(cx, |client, cx| client.dismiss(cx));
            return;
        }
        match &self.screen {
            Screen::Settings(..) => self.close_settings(window, cx),
            Screen::Welcome(_, Step::Place) => self.previous(window, cx),
            Screen::Welcome(_, Step::Name) | Screen::Channel => cx.propagate(),
        }
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> TitleBar {
        let theme = cx.theme();
        let title = match &self.screen {
            Screen::Channel => Some(div().font_weight(FontWeight::SEMIBOLD).child("BukoSpeak").into_any_element()),
            // The way back and the title in one: the arrow lines up on the gutter.
            Screen::Settings(..) => Some(
                // Buttons cover the bar they sit on, which on Windows is the window's caption:
                // else the system takes their clicks for dragging the window.
                div()
                    .occlude()
                    .ml(-theme::CONTROL_INSET)
                    .child(
                        Button::new("back")
                            .custom(
                                ButtonCustomVariant::new(cx)
                                    .foreground(theme.foreground)
                                    .hover(theme.secondary_hover)
                                    .active(theme.secondary_active),
                            )
                            .h(theme::CONTROL)
                            .px(theme::CONTROL_INSET)
                            .rounded(ButtonRounded::Size(theme.radius))
                            .icon(Icon::from(Glyph::Back))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Einstellungen"))
                            .on_click(cx.listener(|main, _, window, cx| main.close_settings(window, cx))),
                    )
                    .into_any_element(),
            ),
            Screen::Welcome(..) => None,
        };
        let settings = matches!(self.screen, Screen::Channel).then(|| {
            div().occlude().child(
                control("settings", Glyph::Settings, "Einstellungen", Tone::Plain, cx)
                    .on_click(cx.listener(|main, _, window, cx| main.open_settings(window, cx))),
            )
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
        let client = self.client.read(cx);
        let message = match (client.call.notice(), client.trouble) {
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
        let certificate = match *self.client.read(cx).call.phase() {
            Phase::Failed(Failure::CertificateChanged { pinned, presented, .. }) => Some(self.render_certificate(pinned, presented, cx)),
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
