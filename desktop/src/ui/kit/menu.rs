use std::cell::Cell;
use std::panic::Location;
use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AlignItems, AnchoredPositionMode, Animation, AnimationExt, AnyElement, App, AppContext, Bounds, Context, DismissEvent, Display, Edges,
    Element, ElementId, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, GlobalElementId, InspectorElementId, InteractiveElement,
    IntoElement, KeyBinding, LayoutId, MouseButton, ParentElement, Pixels, Point, Position, Render, RenderOnce, Role, ScrollHandle,
    SharedString, Size, StatefulInteractiveElement, Style, Styled, Window, actions, anchored, deferred, div, ease_out_quint,
};

use crate::ui::glyph::Glyph;
use crate::ui::kit::{AppIcon, Field, Icon, Size as FieldSize, TextField, TextFieldEvent, h_flex, list_reach, panel_inset, v_flex};
use crate::ui::marks::Mark;
use crate::ui::theme::{self, ActiveTokens, Typeset};

actions!(menu, [Next, Previous, Choose, Close]);

const CONTEXT: &str = "Menu";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("down", Next, Some(CONTEXT)),
        KeyBinding::new("up", Previous, Some(CONTEXT)),
        KeyBinding::new("enter", Choose, Some(CONTEXT)),
        // Typed into the search, where there is one.
        KeyBinding::new("space", Choose, Some("Menu && !search")),
        KeyBinding::new("escape", Close, Some(CONTEXT)),
    ]);
}

type Action = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct Item {
    label: SharedString,
    icon: Option<AppIcon>,
    /// Shown faint at its end, and searched as its label is.
    detail: Option<SharedString>,
    checked: bool,
    disabled: bool,
    action: Action,
}

impl Item {
    pub fn new(label: impl Into<SharedString>, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { label: label.into(), icon: None, detail: None, checked: false, disabled: false, action: Rc::new(action) }
    }

    pub fn icon(mut self, icon: AppIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        [Some(&self.label), self.detail.as_ref()].into_iter().flatten().any(|text| text.to_lowercase().contains(&query))
    }
}

/// What a menu offers, in order.
#[derive(Default)]
pub struct Entries(Vec<Entry>);

impl Entries {
    pub fn item(mut self, item: Item) -> Self {
        self.0.push(Entry::Item(item));
        self
    }

    pub fn separator(mut self) -> Self {
        self.0.push(Entry::Separator);
        self
    }

    pub fn heading(mut self, title: impl Into<SharedString>) -> Self {
        self.0.push(Entry::Heading(title.into()));
        self
    }
}

enum Entry {
    Item(Item),
    Separator,
    Heading(SharedString),
}

impl Entry {
    fn height(&self) -> Pixels {
        match self {
            Self::Item(_) | Self::Heading(_) => theme::menu::ITEM,
            Self::Separator => theme::HAIRLINE + theme::menu::SEPARATOR * 2.,
        }
    }
}

/// The entries shown of `entries` for `query`: the items it matches, a heading only above one of
/// them, and a separator only between them.
fn shown(entries: &[Entry], query: &str) -> Vec<usize> {
    let mut shown = Vec::new();
    let (mut separator, mut heading) = (None, None);
    for (index, entry) in entries.iter().enumerate() {
        match entry {
            Entry::Separator => (separator, heading) = ((!shown.is_empty()).then_some(index), None),
            Entry::Heading(_) => heading = Some(index),
            Entry::Item(item) if item.matches(query) => {
                shown.extend(separator.take().into_iter().chain(heading.take()));
                shown.push(index);
            }
            Entry::Item(_) => {}
        }
    }
    shown
}

fn chosen(entries: &[Entry]) -> Option<usize> {
    entries.iter().position(|entry| matches!(entry, Entry::Item(item) if item.checked))
}

fn anchor(entries: &[Entry]) -> Pixels {
    let anchor = chosen(entries).unwrap_or(0);
    let above: Pixels = entries[..anchor].iter().map(Entry::height).sum();
    let anchor = entries.get(anchor).map_or(Pixels::ZERO, Entry::height);
    theme::HAIRLINE + theme::menu::PAD + above + anchor / 2.
}

fn item_inset() -> Pixels {
    panel_inset() - theme::menu::PAD
}

/// Where the labels of items with a program's icon start, and what is typed into the search.
fn label_inset() -> Pixels {
    item_inset() + theme::app_icon::SM.size + theme::INLINE
}

pub struct Menu {
    entries: Vec<Entry>,
    search: Option<(Entity<TextField>, gpui::Subscription)>,
    highlighted: Option<usize>,
    scroll: ScrollHandle,
    focus: FocusHandle,
    previous: Option<FocusHandle>,
    closed: bool,
    side: Rc<Cell<Side>>,
    _focus_out: gpui::Subscription,
}

impl EventEmitter<DismissEvent> for Menu {}

impl Focusable for Menu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Menu {
    pub fn open(window: &mut Window, cx: &mut App, entries: Entries) -> Entity<Self> {
        Self::opened(None, entries, window, cx)
    }

    /// A menu whose items a search above them narrows down as one types.
    pub fn open_searchable(placeholder: &'static str, window: &mut Window, cx: &mut App, entries: Entries) -> Entity<Self> {
        Self::opened(Some(placeholder), entries, window, cx)
    }

    fn opened(search: Option<&'static str>, Entries(entries): Entries, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let previous = window.focused(cx);
        let menu = cx.new(|cx| {
            let focus = cx.focus_handle();
            // Tab, or the window losing an element, takes the focus away: the menu is done.
            let focus_out = cx.on_focus_out(&focus, window, |menu: &mut Menu, _, window, cx| menu.close(window, cx));
            let search = search.map(|placeholder| {
                let field = cx.new(|cx| TextField::new(String::new(), placeholder, window, cx));
                let typed = cx.subscribe_in(&field, window, |menu: &mut Menu, _, event, window, cx| match event {
                    TextFieldEvent::Changed => {
                        let first = menu.choosable(cx).first().copied();
                        menu.highlight(first, cx);
                        cx.notify();
                    }
                    TextFieldEvent::Confirmed => menu.choose(&Choose, window, cx),
                    TextFieldEvent::Left => {}
                });
                (field, typed)
            });
            let mut menu = Self {
                entries,
                search,
                highlighted: None,
                scroll: ScrollHandle::new(),
                focus,
                previous,
                closed: false,
                side: Rc::default(),
                _focus_out: focus_out,
            };
            let first = chosen(&menu.entries).or_else(|| menu.choosable(cx).first().copied());
            menu.highlight(first, cx);
            menu
        });
        match menu.read(cx).search.as_ref().map(|(field, _)| field.clone()) {
            Some(field) => field.update(cx, |field, cx| field.focus(window, cx)),
            None => window.focus(&menu.focus_handle(cx), cx),
        }
        menu
    }

    /// Offers `entries` from now on, keeping the item highlighted if it is still offered.
    pub fn set_entries(&mut self, Entries(entries): Entries, cx: &mut Context<Self>) {
        let label = |entries: &[Entry], index: usize| match entries.get(index) {
            Some(Entry::Item(item)) => Some(item.label.clone()),
            _ => None,
        };
        let highlighted = self.highlighted.and_then(|index| label(&self.entries, index));
        self.entries = entries;
        let choosable = self.choosable(cx);
        let kept = highlighted
            .and_then(|highlighted| choosable.iter().copied().find(|&index| label(&self.entries, index).as_ref() == Some(&highlighted)));
        self.highlight(kept.or_else(|| choosable.first().copied()), cx);
        cx.notify();
    }

    /// Highlights the item at `index`, scrolled into view.
    fn highlight(&mut self, index: Option<usize>, cx: &App) {
        self.highlighted = index;
        let at = index.and_then(|index| shown(&self.entries, &self.query(cx)).iter().position(|&shown| shown == index));
        if let Some(at) = at {
            self.scroll.scroll_to_item(at);
        }
    }

    fn query(&self, cx: &App) -> String {
        self.search.as_ref().map(|(field, _)| field.read(cx).value().to_owned()).unwrap_or_default()
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Giving the focus back takes it from the menu, which would close it again.
        if self.closed {
            return;
        }
        self.closed = true;
        if self.focus.contains_focused(window, cx)
            && let Some(previous) = &self.previous
        {
            window.focus(previous, cx);
        }
        cx.emit(DismissEvent);
    }

    fn choose_at(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Entry::Item(item)) = self.entries.get(index) else {
            return;
        };
        if item.disabled {
            return;
        }
        let action = item.action.clone();
        // Closed first, so what the item does finds the focus back where the menu was opened.
        self.close(window, cx);
        action(window, cx);
    }

    /// The items shown that can be chosen, in order.
    fn choosable(&self, cx: &App) -> Vec<usize> {
        let enabled = |&index: &usize| matches!(&self.entries[index], Entry::Item(item) if !item.disabled);
        shown(&self.entries, &self.query(cx)).into_iter().filter(enabled).collect()
    }

    fn next(&mut self, _: &Next, _: &mut Window, cx: &mut Context<Self>) {
        let choosable = self.choosable(cx);
        let after = choosable.iter().copied().find(|index| self.highlighted.is_none_or(|highlighted| *index > highlighted));
        self.highlight(after.or_else(|| choosable.first().copied()), cx);
        cx.notify();
    }

    fn previous(&mut self, _: &Previous, _: &mut Window, cx: &mut Context<Self>) {
        let choosable = self.choosable(cx);
        let before = choosable.iter().copied().rev().find(|index| self.highlighted.is_none_or(|highlighted| *index < highlighted));
        self.highlight(before.or_else(|| choosable.last().copied()), cx);
        cx.notify();
    }

    fn choose(&mut self, _: &Choose, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.highlighted {
            self.choose_at(index, window, cx);
        }
    }

    fn close_action(&mut self, _: &Close, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
    }
}

impl Render for Menu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = cx.tokens();
        let query = self.query(cx);
        let shown = shown(&self.entries, &query);
        // A search that finds nothing says so, where what is typed and the items' labels start.
        let nothing = (shown.is_empty() && !query.trim().is_empty())
            .then(|| h_flex().h(theme::menu::ITEM).pl(label_inset()).text_color(tokens.text_faint).child("Nichts gefunden"));
        let entries = shown.into_iter().map(|index| {
            let entry = &self.entries[index];
            match entry {
                Entry::Separator => v_flex()
                    .h(entry.height())
                    .justify_center()
                    .px(item_inset())
                    .child(div().h(theme::HAIRLINE).bg(tokens.hairline))
                    .into_any_element(),
                Entry::Heading(title) => h_flex()
                    .h(entry.height())
                    .px(item_inset())
                    .pt(theme::LINE)
                    .typeset(theme::TEXT_XS)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(tokens.text_faint)
                    .child(title.clone())
                    .into_any_element(),
                Entry::Item(item) => {
                    let highlighted = self.highlighted == Some(index);
                    // The check at the end, where a list row has its menu's mark, so the label
                    // starts where the row's does.
                    h_flex()
                        .id(index)
                        .mark(item.label.clone())
                        .role(Role::MenuItem)
                        .flex_none()
                        .h(entry.height())
                        .px(item_inset())
                        .gap(theme::INLINE)
                        .rounded(theme::RADIUS_CONTROL_SM)
                        .children(item.icon.clone())
                        .child(div().flex_1().min_w_0().truncate().child(item.label.clone()))
                        .children(
                            item.detail
                                .clone()
                                .map(|detail| div().flex_none().typeset(theme::TEXT_SM).text_color(tokens.text_faint).child(detail)),
                        )
                        .children(item.checked.then(|| Icon::new(Glyph::Check).size(theme::ICON_SM)))
                        .map(|row| {
                            if item.disabled {
                                row.opacity(theme::DISABLED_OPACITY)
                            } else {
                                row.cursor_pointer()
                                    .when(highlighted, |row| row.bg(tokens.hover))
                                    .hover(|style| style.bg(tokens.hover))
                                    .on_click(cx.listener(move |menu, _, window, cx| menu.choose_at(index, window, cx)))
                            }
                        })
                        .into_any_element()
                }
            }
        });
        // The magnifier over the items' icons, and what is typed where their labels start.
        let search = self.search.as_ref().map(|(field, _)| {
            let icons = theme::app_icon::SM.size;
            h_flex()
                .flex_none()
                .relative()
                .mb(theme::menu::PAD)
                .child(Field::new(field, FieldSize::Small).pl(label_inset() - theme::field::INSET))
                .child(
                    h_flex()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(item_inset() + (icons - theme::ICON_SM) / 2.)
                        .child(Icon::new(Glyph::Search).size(theme::ICON_SM).color(tokens.text_faint)),
                )
        });
        v_flex()
            .id("menu")
            // Above its backdrop, which closes it on a press: a press on an item is the item's.
            .occlude()
            .role(Role::Menu)
            .key_context(if self.search.is_some() { "Menu search" } else { CONTEXT })
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::next))
            .on_action(cx.listener(Self::previous))
            .on_action(cx.listener(Self::choose))
            .on_action(cx.listener(Self::close_action))
            .relative()
            .min_w(theme::menu::WIDTH)
            .max_h_full()
            .p(theme::menu::PAD)
            .rounded(theme::menu::RADIUS)
            .bg(tokens.floating)
            .border_1()
            .border_color(tokens.edge)
            .shadow(theme::depth::floating(tokens))
            .typeset(theme::TEXT)
            .text_color(tokens.text)
            .children(search)
            .child(
                v_flex()
                    .id("entries")
                    .min_h_0()
                    .max_h(theme::menu::LIST)
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(entries)
                    .children(nothing),
            )
            // Its first frame, before it is placed, is unseen.
            .with_animation("pop", Animation::new(theme::POP).with_easing(ease_out_quint()), {
                let toward = match self.side.get() {
                    Side::Below => -1.,
                    Side::Above => 1.,
                    Side::Over => 0.,
                };
                move |menu, delta| menu.opacity(delta).top(theme::menu::RISE * toward * (1. - delta))
            })
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Place {
    // The popover goes inside the row, which must be positioned relative.
    OverValue,
    /// Below the row, or above it where only there it fits, as wide as the panel the row lies in;
    /// inside the row as for `OverValue`.
    Under,
    At(Point<Pixels>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Side {
    #[default]
    Below,
    Above,
    Over,
}

#[derive(IntoElement)]
pub struct Popover {
    menu: Entity<Menu>,
    place: Place,
}

impl Popover {
    pub fn new(menu: &Entity<Menu>, place: Place) -> Self {
        Self { menu: menu.clone(), place }
    }
}

impl RenderOnce for Popover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let viewport = window.viewport_size();
        let close = |menu: &Entity<Menu>| {
            let menu = menu.clone();
            move |_: &gpui::MouseDownEvent, window: &mut Window, cx: &mut App| menu.update(cx, |menu, cx| menu.close(window, cx))
        };
        let backdrop = deferred(
            anchored().position_mode(AnchoredPositionMode::Window).position(Point::default()).child(
                div()
                    .id("backdrop")
                    .w(viewport.width)
                    .h(viewport.height)
                    .occlude()
                    .on_mouse_down(MouseButton::Left, close(&self.menu))
                    .on_mouse_down(MouseButton::Right, close(&self.menu)),
            ),
        )
        .with_priority(1);
        let (side, anchor) = {
            let menu = self.menu.read(cx);
            (menu.side.clone(), anchor(&menu.entries))
        };
        // No wider than the window inside its gutters: a longer label is cut short.
        let under = matches!(self.place, Place::Under);
        let menu = div()
            .max_w(viewport.width - theme::GUTTER * 2.)
            .max_h(viewport.height - theme::GUTTER * 2.)
            // A column, so the menu spans the width it is given.
            .flex()
            .flex_col()
            .when(under, |menu| menu.w_full())
            .child(self.menu)
            .into_any_element();
        let menu = deferred(Placed { place: self.place, anchor, side, menu }).with_priority(2);
        // Spanning what opened it, whose bounds the menu is placed by: under a row, the panel's
        // width, as far out on each side as the row lies inside the panel.
        let span = div().absolute().inset_0();
        let span = if under { span.left(-list_reach()).right(-list_reach()) } else { span };
        span.child(backdrop).child(menu)
    }
}

struct Placed {
    place: Place,
    anchor: Pixels,
    menu: AnyElement,
    side: Rc<Cell<Side>>,
}

fn position(place: Place, anchor: Pixels, opener: Bounds<Pixels>, size: Size<Pixels>, viewport: Size<Pixels>) -> (Point<Pixels>, Side) {
    let (top, bottom) = (theme::GUTTER, viewport.height - theme::GUTTER);
    let hang = |x: Pixels, below_at: Pixels, above_at: Pixels| {
        let (below, above) = (bottom - below_at, above_at - top);
        if size.height <= below || below >= above { (x, below_at, Side::Below) } else { (x, above_at - size.height, Side::Above) }
    };
    let (x, y, side) = match place {
        Place::OverValue => (opener.right() + list_reach() - size.width, opener.center().y - anchor, Side::Over),
        Place::Under => hang(opener.left(), opener.bottom() + theme::menu::PAD, opener.top() - theme::menu::PAD),
        Place::At(at) => hang(at.x, at.y, at.y),
    };
    let y = y.min(bottom - size.height).max(top);
    let x = x.min(viewport.width - theme::GUTTER - size.width).max(theme::GUTTER);
    (Point { x, y }, side)
}

impl IntoElement for Placed {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Placed {
    type RequestLayoutState = LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, LayoutId) {
        let menu = self.menu.request_layout(window, cx);
        let style = Style {
            position: Position::Absolute,
            inset: Edges::all(Pixels::ZERO.into()),
            display: Display::Flex,
            // Its own height, not the opener's.
            align_items: Some(AlignItems::FlexStart),
            ..Style::default()
        };
        (window.request_layout(style, [menu], cx), menu)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        opener: Bounds<Pixels>,
        menu: &mut LayoutId,
        window: &mut Window,
        cx: &mut App,
    ) {
        let laid = window.layout_bounds(*menu);
        let (origin, side) = position(self.place, self.anchor, opener, laid.size, window.viewport_size());
        self.side.set(side);
        let offset = origin - laid.origin;
        window.with_element_offset(Point { x: offset.x.round(), y: offset.y.round() }, |window| self.menu.prepaint(window, cx));
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut LayoutId,
        (): &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.menu.paint(window, cx);
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods, reason = "a window and its rows are made up to place menus in")]
mod tests {
    use gpui::{point, px, size};

    use super::*;

    const WINDOW: Size<Pixels> = Size { width: px(400.), height: px(600.) };
    const MENU: Size<Pixels> = Size { width: px(200.), height: px(150.) };

    fn row(top: f32) -> Bounds<Pixels> {
        Bounds { origin: point(px(30.), px(top)), size: size(px(340.), px(44.)) }
    }

    #[test]
    fn over_a_row_s_end_the_menu_ends_on_the_panel_s_end_and_puts_its_chosen_item_on_the_row() {
        let anchor = px(50.);
        let (at, side) = position(Place::OverValue, anchor, row(200.), MENU, WINDOW);
        assert_eq!(side, Side::Over);
        assert_eq!(at, point(px(370.) + list_reach() - MENU.width, px(222.) - anchor));
    }

    #[test]
    fn over_a_row_by_the_window_s_edge_it_moves_inside_the_window() {
        let (at, _) = position(Place::OverValue, px(14.), row(540.), MENU, WINDOW);
        assert_eq!(at.y + MENU.height, WINDOW.height - theme::GUTTER);
        let (at, _) = position(Place::OverValue, px(140.), row(10.), MENU, WINDOW);
        assert_eq!(at.y, theme::GUTTER);
    }

    #[test]
    fn under_a_row_it_spans_the_row_s_panel_below_or_where_only_there_it_fits_above() {
        let panel = |top| Bounds { origin: point(px(30.) - list_reach(), px(top)), size: size(px(340.) + list_reach() * 2., px(44.)) };
        let (at, side) = position(Place::Under, Pixels::ZERO, panel(100.), MENU, WINDOW);
        assert_eq!((at, side), (point(panel(100.).left(), px(144.) + theme::menu::PAD), Side::Below));
        let (at, side) = position(Place::Under, Pixels::ZERO, panel(500.), MENU, WINDOW);
        assert_eq!((at, side), (point(panel(500.).left(), px(500.) - theme::menu::PAD - MENU.height), Side::Above));
    }

    #[test]
    fn at_a_point_it_hangs_below_or_where_only_there_it_fits_above() {
        let (at, side) = position(Place::At(point(px(100.), px(100.))), Pixels::ZERO, Bounds::default(), MENU, WINDOW);
        assert_eq!((at, side), (point(px(100.), px(100.)), Side::Below));
        let (at, side) = position(Place::At(point(px(100.), px(550.))), Pixels::ZERO, Bounds::default(), MENU, WINDOW);
        assert_eq!((at, side), (point(px(100.), px(550.) - MENU.height), Side::Above));
    }

    #[test]
    fn at_a_point_by_the_edge_it_stays_in_the_window() {
        let (at, side) = position(Place::At(point(px(390.), px(10.))), Pixels::ZERO, Bounds::default(), MENU, WINDOW);
        assert_eq!(side, Side::Below);
        assert_eq!(at, point(WINDOW.width - theme::GUTTER - MENU.width, theme::GUTTER));
    }

    #[test]
    fn the_anchor_is_the_middle_of_the_first_chosen_item_else_of_the_first() {
        let entries = |checked| {
            vec![
                Entry::Item(Item::new("first", |_, _| {})),
                Entry::Separator,
                Entry::Heading("vendor".into()),
                Entry::Item(Item::new("chosen", |_, _| {}).checked(checked)),
            ]
        };
        let top = theme::HAIRLINE + theme::menu::PAD;
        let above = theme::menu::ITEM + (theme::HAIRLINE + theme::menu::SEPARATOR * 2.) + theme::menu::ITEM;
        assert_eq!(anchor(&entries(true)), top + above + theme::menu::ITEM / 2.);
        assert_eq!(anchor(&entries(false)), top + theme::menu::ITEM / 2.);
    }

    #[test]
    fn a_heading_shows_only_above_an_item_shown_and_a_separator_only_between_them() {
        let item = |label: &'static str| Entry::Item(Item::new(label, |_, _| {}));
        let detailed = Entry::Item(Item::new("Google Chrome", |_, _| {}).detail("chrome.exe"));
        let entries = vec![Entry::Heading("Spiele".into()), Entry::Separator, Entry::Heading("Andere".into()), item("Spotify"), detailed];
        assert_eq!(shown(&entries, ""), [2, 3, 4]);
        assert_eq!(shown(&entries, "CHROME.exe"), [2, 4]);
        assert_eq!(shown(&entries, "nothing"), Vec::<usize>::new());
        let both =
            vec![Entry::Heading("Spiele".into()), item("Fortnite"), Entry::Separator, Entry::Heading("Andere".into()), item("Spotify")];
        assert_eq!(shown(&both, ""), [0, 1, 2, 3, 4]);
        assert_eq!(shown(&both, "spot"), [3, 4]);
    }
}
