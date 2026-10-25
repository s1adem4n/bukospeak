//! The settings: groups of rows, a label and its control each. The fields are kept when the
//! settings close; the audio's choices at once, as the call takes them at once.

use bukospeak_desktop::audio::{self, Direction};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Pixels, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, px,
};

use crate::ui::client::Client;
use crate::ui::{Form, Main, Screen, edit_menu, theme};

/// How far a row's label lies inside the card, and the group's title with it.
const ROW_PAD: Pixels = px(12.);

/// A device to choose: the system's default for None.
#[derive(Clone, Debug, PartialEq)]
struct Choice {
    id: Option<String>,
    title: SharedString,
    /// Kept but not there now: shown as chosen, not to be chosen again.
    missing: bool,
}

impl SelectItem for Choice {
    type Value = Option<String>;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &Option<String> {
        &self.id
    }

    fn disabled(&self) -> bool {
        self.missing
    }
}

/// The choices for a direction from its devices' ids and names: the default first, then the
/// devices there are, then the one kept if it isn't among them.
fn choices(devices: impl IntoIterator<Item = (String, String)>, kept: Option<&str>) -> Vec<Choice> {
    let standard = Choice { id: None, title: "Standard".into(), missing: false };
    let there = devices.into_iter().map(|(id, name)| Choice { id: Some(id), title: name.into(), missing: false });
    let mut choices: Vec<Choice> = std::iter::once(standard).chain(there).collect();
    if let Some(kept) = kept
        && !choices.iter().any(|choice| choice.id.as_deref() == Some(kept))
    {
        choices.push(Choice { id: Some(kept.to_owned()), title: "Nicht angeschlossen".into(), missing: true });
    }
    choices
}

type DeviceSelect = Entity<SelectState<Vec<Choice>>>;

/// The settings' choice of devices, filled in once the system listed them.
pub(super) struct Audio {
    input: DeviceSelect,
    output: DeviceSelect,
    _events: [Subscription; 2],
    _listing: Task<()>,
}

impl Main {
    pub(super) fn audio(&self, window: &mut Window, cx: &mut Context<Self>) -> Audio {
        let select = |direction: Direction, window: &mut Window, cx: &mut Context<Self>| {
            let kept = self.client.read(cx).settings.device_id(direction).map(str::to_owned);
            let select = cx.new(|cx| {
                // Until the devices are listed only the kept one is known, by its id.
                let known = Choice { id: kept.clone(), title: kept.clone().unwrap_or_else(|| "Standard".into()).into(), missing: false };
                let mut state = SelectState::new(vec![known], None, window, cx);
                state.set_selected_value(&kept, window, cx);
                state
            });
            let events = cx.subscribe_in(&select, window, move |main, _, event: &SelectEvent<Vec<Choice>>, _, cx| {
                let SelectEvent::Confirm(Some(id)) = event else { return };
                main.client.update(cx, |client, cx| client.set_device(direction, id.clone(), cx));
            });
            (select, events)
        };
        let (input, input_events) = select(Direction::Input, window, cx);
        let (output, output_events) = select(Direction::Output, window, cx);
        let listing = cx.spawn_in(window, async move |this, cx| {
            let list = |direction| {
                audio::devices(direction).unwrap_or_else(|error| {
                    tracing::warn!("{error}");
                    Vec::new()
                })
            };
            // Listing asks the sound server: not on the window's thread.
            let (inputs, outputs) = cx.background_spawn(async move { (list(Direction::Input), list(Direction::Output)) }).await;
            // The settings may have closed before the system answered.
            let _ = this.update_in(cx, |main, window, cx| {
                let Screen::Settings(_, audio) = &main.screen else { return };
                for (select, devices, direction) in
                    [(&audio.input, &inputs, Direction::Input), (&audio.output, &outputs, Direction::Output)]
                {
                    let kept = main.client.read(cx).settings.device_id(direction).map(str::to_owned);
                    select.update(cx, |select, cx| {
                        select.set_items(
                            choices(devices.iter().map(|device| (device.id.to_string(), device.name.clone())), kept.as_deref()),
                            window,
                            cx,
                        );
                        select.set_selected_value(&kept, window, cx);
                    });
                }
            });
        });
        Audio { input, output, _events: [input_events, output_events], _listing: listing }
    }

    pub(super) fn render_settings(&self, form: &Form, audio: &Audio, cx: &mut Context<Self>) -> impl IntoElement {
        let field = |input: &Entity<InputState>| Input::new(input).appearance(false).large().context_menu(edit_menu).into_any_element();
        let device = |id: &'static str, select: &DeviceSelect| Select::new(select).id(id).appearance(false).large().into_any_element();
        let settings = &self.client.read(cx).settings;
        let switch = |id: &'static str, on: bool, set: fn(&mut Client, bool, &mut Context<Client>)| {
            let client = self.client.downgrade();
            div()
                .pr_2()
                .child(Switch::new(id).checked(on).on_change(move |on, _, cx| {
                    // The window may be gone with the click.
                    let _ = client.update(cx, |client, cx| set(client, *on, cx));
                }))
                .into_any_element()
        };
        div().id("settings").flex_1().min_h_0().overflow_y_scroll().child(
            v_flex()
                .w_full()
                .max_w(theme::SETTINGS)
                .mx_auto()
                .px(theme::GUTTER)
                .pt_2()
                .pb_6()
                .gap_6()
                .child(group("Profil", vec![Row::field("Name", field(&form.name))], cx))
                .child(group(
                    "Server",
                    vec![Row::field("Adresse", field(&form.server)), Row::field("Raumschlüssel", field(&form.room_key))],
                    cx,
                ))
                .child(group(
                    "Audio",
                    vec![
                        Row::field("Mikrofon", device("input", &audio.input)),
                        Row::field("Ausgabe", device("output", &audio.output)),
                        Row::switch("Rauschunterdrückung", switch("noise", settings.noise_suppression, Client::set_noise_suppression)),
                        Row::switch("Echounterdrückung", switch("echo", settings.echo_cancellation, Client::set_echo_cancellation)),
                    ],
                    cx,
                )),
        )
    }
}

/// A row of a group: a field beside its label, or a switch at the end of the row.
struct Row {
    label: &'static str,
    control: AnyElement,
    switch: bool,
}

impl Row {
    fn field(label: &'static str, control: AnyElement) -> Self {
        Self { label, control, switch: false }
    }

    fn switch(label: &'static str, control: AnyElement) -> Self {
        Self { label, control, switch: true }
    }
}

/// A titled card of rows; the fields line up on one column.
fn group(title: &'static str, rows: Vec<Row>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .gap_2()
        .child(div().px(ROW_PAD).text_size(theme::TEXT_SM).font_weight(FontWeight::MEDIUM).text_color(theme.muted_foreground).child(title))
        .child(v_flex().rounded(theme.radius_lg).border_1().border_color(theme.border).bg(theme.muted).children(
            rows.into_iter().enumerate().map(|(index, row)| {
                let label = div().child(row.label);
                h_flex().h(px(44.)).pl(ROW_PAD).pr_1().gap_3().when(index > 0, |row| row.border_t_1().border_color(theme.border)).map(
                    |line| match row.switch {
                        true => line.child(label.flex_1()).child(row.control),
                        false => line.child(label.w(px(104.)).flex_none()).child(div().flex_1().min_w_0().child(row.control)),
                    },
                )
            }),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_comes_first_and_a_kept_device_not_there_last() {
        let ids = |kept: Option<&str>| {
            let choices = choices([("first".to_owned(), "First".to_owned())], kept);
            choices.into_iter().map(|choice| (choice.id, choice.missing)).collect::<Vec<_>>()
        };
        let there = [(None, false), (Some("first".to_owned()), false)];
        assert_eq!(ids(None), there);
        assert_eq!(ids(Some("first")), there);
        assert_eq!(ids(Some("second")), [there[0].clone(), there[1].clone(), (Some("second".to_owned()), true)]);
    }
}
