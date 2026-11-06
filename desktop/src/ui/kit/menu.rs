//! A menu: items to choose from, shown beneath what opened it or where a right click was. Open,
//! it has the focus: the arrows move through it, Enter chooses, Escape closes it, and the focus
//! goes back where it was. A click beside it closes it and does nothing else.

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
use crate::ui::kit::{Icon, h_flex, v_flex};
use crate::ui::marks::Mark;
use crate::ui::theme::{self, ActiveTokens, Typeset};

actions!(
    menu,
    [
        /// The item below the highlighted one.
        Next,
        /// The item above it.
        Previous,
        /// Chooses the highlighted item.
        Choose,
        /// Closes the menu, choosing nothing.
        Close
    ]
);

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

/// An item: what it says, whether it is the one chosen now, and what choosing it does.
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
    /// The title of the items after it, which aren't the menu's own: a vendor's.
    Heading(SharedString),
}

pub struct Menu {
    entries: Vec<Entry>,
    /// Whether any item shows a check, so the labels of all line up after its place.
    checks: bool,
    highlighted: Option<usize>,
    focus: FocusHandle,
    /// Where the focus was before the menu took it, and goes back to.
    previous: Option<FocusHandle>,
    /// Whether it was closed, after which its owner drops it.
    closed: bool,
    /// The side of what opened it it came out on, as [`Popover`] placed it last.
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
    /// Opens a menu of what `build` puts in it, and gives it the focus; its owner shows it
    /// with [`Popover`] until it emits [`DismissEvent`].
    pub fn open(window: &mut Window, cx: &mut App, build: impl FnOnce(Menu) -> Menu) -> Entity<Self> {
        let previous = window.focused(cx);
        let menu = cx.new(|cx| {
            let focus = cx.focus_handle();
            // Tab, or the window losing an element, takes the focus away: the menu is done.
            let focus_out = cx.on_focus_out(&focus, window, |menu: &mut Menu, _, window, cx| menu.close(window, cx));
            build(Self {
                entries: Vec::new(),
                checks: false,
                highlighted: None,
                focus,
                previous,
                closed: false,
                side: Rc::default(),
                _focus_out: focus_out,
            })
        });
        window.focus(&menu.focus_handle(cx), cx);
        menu
    }

    pub fn item(mut self, item: Item) -> Self {
        self.checks |= item.checked;
        self.entries.push(Entry::Item(item));
        self
    }

    pub fn separator(mut self) -> Self {
        self.entries.push(Entry::Separator);
        self
    }

    /// Titles the items that follow, lined up with their labels.
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

    /// The items that can be chosen, by their index among the entries.
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
            Entry::Separator => div().my_1().mx_2().h(theme::HAIRLINE).bg(tokens.hairline).into_any_element(),
            Entry::Heading(title) => h_flex()
                .px_2()
                .pt_1()
                .gap_2()
                .h(theme::MENU_ITEM)
                .typeset(theme::TEXT_XS)
                .font_weight(FontWeight::MEDIUM)
                .text_color(tokens.text_faint)
                .when(self.checks, |heading| heading.child(div().flex_none().size(theme::ICON_SM)))
                .child(title.clone())
                .into_any_element(),
            Entry::Item(item) => {
                let highlighted = self.highlighted == Some(index);
                h_flex()
                    .id(index)
                    .mark(item.label.clone())
                    .role(Role::MenuItem)
                    .h(theme::MENU_ITEM)
                    .px_2()
                    .gap_2()
                    .rounded(theme::RADIUS_CONTROL_SM)
                    .when(self.checks, |row| {
                        row.child(div().size(theme::ICON_SM).children(item.checked.then(|| Icon::new(Glyph::Check).size(theme::ICON_SM))))
                    })
                    .child(div().flex_1().min_w_0().truncate().child(item.label.clone()))
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
            // Comes on from what opened it, settling the last of the gap between. Its first frame,
            // before it is placed, is unseen.
            .with_animation("pop", Animation::new(theme::POP).with_easing(ease_out_quint()), {
                let toward = match self.side.get() {
                    Side::Below => -1.,
                    Side::Above => 1.,
                };
                move |menu, delta| menu.opacity(delta).top(theme::MENU_GAP * toward * (1. - delta))
            })
    }
}

/// Where an open menu shows.
#[derive(Clone, Copy, Debug)]
pub enum Place {
    /// Beneath the element the popover is put in, their right edges in line; above it where
    /// there is no room beneath. That element is positioned for it.
    BelowEnd,
    /// At a point of the window, its top left corner there: a menu a right click opens.
    At(Point<Pixels>),
}

/// Which side of what opened it a menu came out on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Side {
    #[default]
    Below,
    Above,
}

/// An open menu, drawn above everything in the window, over a backdrop that takes a click
/// beside it and closes it.
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
        let menu = deferred(Placed { place: self.place, side: self.menu.read(cx).side.clone(), menu: self.menu.into_any_element() })
            .with_priority(2);
        // Spanning what opened it, whose bounds the menu is placed by.
        div().absolute().inset_0().child(backdrop).child(menu)
    }
}

/// A menu laid out at its own size and placed by what opened it: beneath, in line with its
/// end, or above where only there it fits, a gap away; at the window's edge where it would
/// cross it.
struct Placed {
    place: Place,
    menu: AnyElement,
    /// Told the side it came out on, which its motion comes from.
    side: Rc<Cell<Side>>,
}

/// Where a menu of `size` goes for `place`, beside `opener` in a window of `viewport`, and on
/// which side.
fn position(place: Place, opener: Bounds<Pixels>, size: Size<Pixels>, viewport: Size<Pixels>) -> (Point<Pixels>, Side) {
    let (target, gap, x) = match place {
        Place::BelowEnd => (opener, theme::MENU_GAP, opener.right() - size.width),
        Place::At(at) => (Bounds { origin: at, size: Size::default() }, Pixels::ZERO, at.x),
    };
    let (top, bottom) = (theme::GUTTER, viewport.height - theme::GUTTER);
    let below = bottom - (target.bottom() + gap);
    let above = target.top() - gap - top;
    let side = if size.height <= below || below >= above { Side::Below } else { Side::Above };
    let y = match side {
        Side::Below => target.bottom() + gap,
        Side::Above => target.top() - gap - size.height,
    };
    // Taller than the room on either side, it lies along the window's top edge.
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
        let (origin, side) = position(self.place, opener, laid.size, window.viewport_size());
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
        Bounds { origin: point(px(20.), px(top)), size: size(px(360.), px(44.)) }
    }

    #[test]
    fn a_menu_hangs_below_its_row_in_line_with_its_end() {
        let (at, side) = position(Place::BelowEnd, row(100.), MENU, WINDOW);
        assert_eq!(side, Side::Below);
        assert_eq!(at, point(px(380.) - MENU.width, px(144.) + theme::MENU_GAP));
    }

    #[test]
    fn without_room_below_it_stands_above_its_row_and_leaves_it_free() {
        let (at, side) = position(Place::BelowEnd, row(500.), MENU, WINDOW);
        assert_eq!(side, Side::Above);
        assert_eq!(at.y + MENU.height, px(500.) - theme::MENU_GAP);
    }

    #[test]
    fn taller_than_either_side_it_takes_the_larger_and_stays_in_the_window() {
        let tall = size(MENU.width, px(400.));
        let (at, side) = position(Place::BelowEnd, row(350.), tall, WINDOW);
        assert_eq!(side, Side::Above);
        assert_eq!(at.y, theme::GUTTER);
        let (at, side) = position(Place::BelowEnd, row(150.), tall, WINDOW);
        assert_eq!(side, Side::Below);
        assert_eq!(at.y + tall.height, WINDOW.height - theme::GUTTER);
    }

    #[test]
    fn at_a_point_by_the_edge_it_stays_in_the_window() {
        let (at, side) = position(Place::At(point(px(390.), px(10.))), Bounds::default(), MENU, WINDOW);
        assert_eq!(side, Side::Below);
        assert_eq!(at, point(WINDOW.width - theme::GUTTER - MENU.width, theme::GUTTER));
    }
}
