//! A menu: items to choose from, shown beneath what opened it or where a right click was. Open,
//! it has the focus: the arrows move through it, Enter chooses, Escape closes it, and the focus
//! goes back where it was. A click beside it closes it and does nothing else.

use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::{
    Anchor, AnchoredPositionMode, App, AppContext, BoxShadow, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyBinding, MouseButton, ParentElement, Pixels, Point, Render, RenderOnce, Role, SharedString,
    StatefulInteractiveElement, Styled, Window, actions, anchored, deferred, div, px,
};

use crate::ui::glyph::Glyph;
use crate::ui::kit::{Icon, h_flex, v_flex};
use crate::ui::theme::{self, ActiveTokens};

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
            build(Self { entries: Vec::new(), checks: false, highlighted: None, focus, previous, closed: false, _focus_out: focus_out })
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
            Entry::Separator => div().my_1().mx_2().h(px(1.)).bg(tokens.hairline).into_any_element(),
            Entry::Item(item) => {
                let highlighted = self.highlighted == Some(index);
                h_flex()
                    .id(index)
                    .role(Role::MenuItem)
                    .h(theme::MENU_ITEM)
                    .px_2()
                    .gap_2()
                    .rounded(theme::RADIUS_SMALL)
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
            .role(Role::Menu)
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::next))
            .on_action(cx.listener(Self::previous))
            .on_action(cx.listener(Self::choose))
            .on_action(cx.listener(Self::close_action))
            .min_w(theme::MENU)
            .p(theme::MENU_PAD)
            .rounded(theme::RADIUS_CONTROL)
            .bg(tokens.floating)
            .border_1()
            .border_color(tokens.edge)
            .shadow(vec![BoxShadow::new(px(0.), px(4.), tokens.shadow).blur_radius(px(12.))])
            .text_size(theme::TEXT)
            .text_color(tokens.text)
            .children(entries)
    }
}

/// Where an open menu shows.
#[derive(Clone, Copy, Debug)]
pub enum Place {
    /// Beneath the element the popover is put in, which is positioned for it, their right edges
    /// in line.
    BelowEnd,
    /// At a point of the window, its top left corner there: a menu a right click opens.
    At(Point<Pixels>),
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
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
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
        let margin = anchored().snap_to_window_with_margin(theme::GAP);
        let menu = match self.place {
            Place::BelowEnd => margin.anchor(Anchor::TopRight).child(div().mt(theme::MENU_GAP).child(self.menu)),
            Place::At(point) => margin.position_mode(AnchoredPositionMode::Window).position(point).child(self.menu),
        };
        let menu = deferred(menu).with_priority(2);
        match self.place {
            // A point at the element's bottom right corner, which the menu hangs from.
            Place::BelowEnd => div().absolute().right_0().bottom_0().child(backdrop).child(menu),
            Place::At(_) => div().absolute().child(backdrop).child(menu),
        }
    }
}
