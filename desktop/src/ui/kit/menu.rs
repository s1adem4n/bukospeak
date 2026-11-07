use std::cell::Cell;
use std::panic::Location;
use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AlignItems, AnchoredPositionMode, Animation, AnimationExt, AnyElement, App, AppContext, Bounds, Context, DismissEvent, Display, Edges,
    Element, ElementId, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, GlobalElementId, InspectorElementId, InteractiveElement,
    IntoElement, KeyBinding, LayoutId, MouseButton, ParentElement, Pixels, Point, Position, Render, RenderOnce, Role, SharedString, Size,
    StatefulInteractiveElement, Style, Styled, Window, actions, anchored, deferred, div, ease_out_quint,
};

use crate::ui::glyph::Glyph;
use crate::ui::kit::{Icon, h_flex, list_reach, panel_inset, v_flex};
use crate::ui::marks::Mark;
use crate::ui::theme::{self, ActiveTokens, Typeset};

actions!(menu, [Next, Previous, Choose, Close]);

const CONTEXT: &str = "Menu";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("down", Next, Some(CONTEXT)),
        KeyBinding::new("up", Previous, Some(CONTEXT)),
        KeyBinding::new("enter", Choose, Some(CONTEXT)),
        KeyBinding::new("space", Choose, Some(CONTEXT)),
        KeyBinding::new("escape", Close, Some(CONTEXT)),
    ]);
}

type Action = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct Item {
    label: SharedString,
    checked: bool,
    disabled: bool,
    action: Action,
}

impl Item {
    pub fn new(label: impl Into<SharedString>, action: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { label: label.into(), checked: false, disabled: false, action: Rc::new(action) }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
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
            Self::Item(_) | Self::Heading(_) => theme::MENU_ITEM,
            Self::Separator => theme::HAIRLINE + theme::MENU_SEPARATOR * 2.,
        }
    }
}

fn chosen(entries: &[Entry]) -> Option<usize> {
    entries.iter().position(|entry| matches!(entry, Entry::Item(item) if item.checked))
}

fn anchor(entries: &[Entry]) -> Pixels {
    let anchor = chosen(entries).unwrap_or(0);
    let above: Pixels = entries[..anchor].iter().map(Entry::height).sum();
    let anchor = entries.get(anchor).map_or(Pixels::ZERO, Entry::height);
    theme::HAIRLINE + theme::MENU_PAD + above + anchor / 2.
}

fn item_inset() -> Pixels {
    panel_inset() - theme::MENU_PAD
}

pub struct Menu {
    entries: Vec<Entry>,
    highlighted: Option<usize>,
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
    pub fn open(window: &mut Window, cx: &mut App, build: impl FnOnce(Menu) -> Menu) -> Entity<Self> {
        let previous = window.focused(cx);
        let menu = cx.new(|cx| {
            let focus = cx.focus_handle();
            // Tab, or the window losing an element, takes the focus away: the menu is done.
            let focus_out = cx.on_focus_out(&focus, window, |menu: &mut Menu, _, window, cx| menu.close(window, cx));
            let menu = build(Self {
                entries: Vec::new(),
                highlighted: None,
                focus,
                previous,
                closed: false,
                side: Rc::default(),
                _focus_out: focus_out,
            });
            Self { highlighted: chosen(&menu.entries), ..menu }
        });
        window.focus(&menu.focus_handle(cx), cx);
        menu
    }

    pub fn item(mut self, item: Item) -> Self {
        self.entries.push(Entry::Item(item));
        self
    }

    pub fn separator(mut self) -> Self {
        self.entries.push(Entry::Separator);
        self
    }

    pub fn heading(mut self, title: impl Into<SharedString>) -> Self {
        self.entries.push(Entry::Heading(title.into()));
        self
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

    fn choosable(&self) -> impl DoubleEndedIterator<Item = usize> + '_ {
        self.entries.iter().enumerate().filter_map(|(index, entry)| matches!(entry, Entry::Item(item) if !item.disabled).then_some(index))
    }

    fn next(&mut self, _: &Next, _: &mut Window, cx: &mut Context<Self>) {
        let after = self.choosable().find(|index| self.highlighted.is_none_or(|highlighted| *index > highlighted));
        self.highlighted = after.or_else(|| self.choosable().next());
        cx.notify();
    }

    fn previous(&mut self, _: &Previous, _: &mut Window, cx: &mut Context<Self>) {
        let before = self.choosable().rev().find(|index| self.highlighted.is_none_or(|highlighted| *index < highlighted));
        self.highlighted = before.or_else(|| self.choosable().next_back());
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
        let entries = self.entries.iter().enumerate().map(|(index, entry)| match entry {
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
                // The check at the end, where a list row has its menu's mark, so the label starts
                // where the row's does.
                h_flex()
                    .id(index)
                    .mark(item.label.clone())
                    .role(Role::MenuItem)
                    .h(entry.height())
                    .px(item_inset())
                    .gap(theme::GAP)
                    .rounded(theme::RADIUS_CONTROL_SM)
                    .child(div().flex_1().min_w_0().truncate().child(item.label.clone()))
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
        });
        v_flex()
            .id("menu")
            // Above its backdrop, which closes it on a press: a press on an item is the item's.
            .occlude()
            .role(Role::Menu)
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::next))
            .on_action(cx.listener(Self::previous))
            .on_action(cx.listener(Self::choose))
            .on_action(cx.listener(Self::close_action))
            .relative()
            .min_w(theme::MENU)
            .p(theme::MENU_PAD)
            .rounded(theme::RADIUS_MENU)
            .bg(tokens.floating)
            .border_1()
            .border_color(tokens.edge)
            .shadow(theme::depth::floating(tokens))
            .typeset(theme::TEXT)
            .text_color(tokens.text)
            .children(entries)
            // Its first frame, before it is placed, is unseen.
            .with_animation("pop", Animation::new(theme::POP).with_easing(ease_out_quint()), {
                let toward = match self.side.get() {
                    Side::Below => -1.,
                    Side::Above => 1.,
                    Side::Over => 0.,
                };
                move |menu, delta| menu.opacity(delta).top(theme::MENU_RISE * toward * (1. - delta))
            })
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Place {
    // The popover goes inside the row, which must be positioned relative.
    OverValue,
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
        let menu = div().max_w(viewport.width - theme::GUTTER * 2.).child(self.menu).into_any_element();
        let menu = deferred(Placed { place: self.place, anchor, side, menu }).with_priority(2);
        // Spanning what opened it, whose bounds the menu is placed by.
        div().absolute().inset_0().child(backdrop).child(menu)
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
    let (x, y, side) = match place {
        Place::OverValue => (opener.right() + list_reach() - size.width, opener.center().y - anchor, Side::Over),
        Place::At(at) => {
            let (below, above) = (bottom - at.y, at.y - top);
            if size.height <= below || below >= above { (at.x, at.y, Side::Below) } else { (at.x, at.y - size.height, Side::Above) }
        }
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
        let top = theme::HAIRLINE + theme::MENU_PAD;
        let above = theme::MENU_ITEM + (theme::HAIRLINE + theme::MENU_SEPARATOR * 2.) + theme::MENU_ITEM;
        assert_eq!(anchor(&entries(true)), top + above + theme::MENU_ITEM / 2.);
        assert_eq!(anchor(&entries(false)), top + theme::MENU_ITEM / 2.);
    }
}
