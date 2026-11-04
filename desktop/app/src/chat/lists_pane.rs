//! The Lists pane (M85 lists §3): one of the owner's lists at the chat's end, under the window's
//! header. A split view around the chat's body holds it, in a breakpoint bin that collapses the
//! split below 600sp, where the pane lies over the chat and shows only when asked. Its head picks
//! the list, opens the list's menu behind More (`list_more`), and gives its folder and counts; its
//! list draws the lines `checklists::panel` gives, spliced where they changed, an item being
//! reworded as its two entries, each item with its menu (`list_menu`) and its moves; Add an item
//! sits at its foot. Whether it shows is the owner's last open or close
//! (`checklists::pane::PaneOpen`), which the session keeps. Only stock widgets, markup off before
//! any text, and no list edit is an exported action: the pane hands each to the session through
//! `Lister`.

use super::entries::copy_text;
use super::list_menu::{self, ItemOffers};
use super::list_more::{self, ListActs};
use super::list_reword::Reworder;
use super::list_rows::{self, moves_on, on_click_or_key, rewords_on};
use super::lists::{act, ListAct, Lister, PANE_MARK};
use adw::prelude::*;
use fermix_client::chat_wire::protocol::{List, ListEntry, ListOp, ListRow};
use fermix_client::checklists::card::Settling;
use fermix_client::checklists::draft::Place;
use fermix_client::checklists::index::Folders;
use fermix_client::checklists::library::Library;
use fermix_client::checklists::pane::{
    drawing, folder_op, list_offers, news, rewording, Folds, ListOffers, PaneBody, PaneDrawing,
    PaneList, PaneOpen,
};
use fermix_client::checklists::panel::{self, add_place, rewords, splice, Line, Splice, Steps};
use fermix_client::checklists::words::{
    self, ADD_ITEM, HIDE_LISTS, LISTS, LIST_REMOVED, NO_LISTS, RECENTLY_REMOVED,
};
use gtk::gio;
use gtk::glib::{self, BoxedAnyObject};
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::{Rc, Weak};

/// The pane's share of the chat: 0.4 of it, from 280 to 420sp, so 330 at the default window.
const FRACTION: f64 = 0.4;
const MIN_WIDTH: f64 = 280.0;
const MAX_WIDTH: f64 = 420.0;
/// Narrower than this, the chat collapses the split and the pane lies over it.
const NARROW: &str = "max-width: 600sp";
/// A breakpoint bin needs a minimum size; the window's smallest fits it.
const MIN_SIZE: (i32, i32) = (360, 300);
/// The tallest the picker's list grows before it scrolls.
const PICKS_HEIGHT: i32 = 420;

/// One line of the list's model, with the list it is drawn for.
struct PaneLine {
    list_id: String,
    line: Line,
}

/// One row of the picker: the list it shows and its folder, which the header function reads.
struct Pick {
    list_id: String,
    folder: Option<String>,
}

pub struct ListsPane {
    /// The chat's body with the pane beside or over it: what Chat shows.
    pub widget: adw::BreakpointBin,
    split: adw::OverlaySplitView,
    /// The pane itself, a group labelled "Lists".
    column: gtk::Box,
    head: Head,
    body: gtk::Stack,
    list: gtk::ListBox,
    lines: gio::ListStore,
    foot: Foot,
    lister: Lister,
    open: Cell<PaneOpen>,
    /// The list last asked for, by a card or the picker.
    asked: RefCell<Option<String>>,
    /// Each list's folded sections and whether its Done is open, kept while the pane lives.
    folds: RefCell<Folds>,
    /// The rows ticked a moment ago, which keep their place while they settle.
    settling: Rc<RefCell<Settling>>,
    drawn: RefCell<Option<PaneDrawing>>,
    /// The list drawn, as the library holds it: Add an item places its item by it.
    held: RefCell<Option<List>>,
    /// A pull of the shown list waits for an idle: the pane draws while the session holds the
    /// lists.
    pulling: Cell<bool>,
    /// Where the focus was in the chat before the pane opened, for its close to give it back.
    return_to: glib::WeakRef<gtk::Widget>,
    /// Where the focus goes when the pane hides with it and that is not on screen: the composer.
    composer: gtk::Widget,
    /// The item being reworded, if one is.
    reworder: Reworder,
    /// The item last reworded: its row takes the focus when its rewording ends by Return or
    /// Escape, and the selection when it was selected as the rewording began.
    reworded: RefCell<Option<String>>,
    reselect: Cell<bool>,
    /// While a move within the list is drawn, the line that takes the focus: the moved item in
    /// its new place, or the folded heading it went under.
    moving: RefCell<Option<String>>,
}

/// The head (§3.3): the picker, More and Hide lists, then the folder and counts.
struct Head {
    picker: gtk::MenuButton,
    title: gtk::Label,
    picks: gtk::ListBox,
    picked: Rc<RefCell<Vec<Pick>>>,
    /// The index's folders and the shown list the picks were drawn for: a tick or a push that
    /// changes neither leaves the picks, and an open picker, as they are.
    picks_of: RefCell<Option<(Folders, String)>>,
    /// Opens the list's menu; shown while a list is drawn that is not removed.
    more: gtk::MenuButton,
    /// The list and the offers More's menu was last made for, none while it is being made: a
    /// draw that changes either closes the menu while it shows.
    menu_of: RefCell<Option<(String, ListOffers)>>,
    hide: gtk::Button,
    counts: gtk::Label,
}

/// The foot (§3.5, §3.8): the engine's sentence for a refused edit, over Add an item.
struct Foot {
    widget: gtk::Box,
    refusal: gtk::Label,
    add: adw::EntryRow,
}

impl ListsPane {
    /// The pane beside `chat`, the chat's body, closed until the session says otherwise.
    /// `composer` takes the focus when the pane hides with it and its place is gone.
    pub fn new(
        lister: &Lister,
        chat: &impl IsA<gtk::Widget>,
        composer: &impl IsA<gtk::Widget>,
    ) -> Rc<ListsPane> {
        let head = Head::new();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .activate_on_single_click(false)
            .css_classes(["boxed-list"])
            .valign(gtk::Align::Start)
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        let foot = Foot::new();
        let body = body_stack(&list);
        let column = column(&head, &body, &foot);
        let (widget, split) = split_around(chat, &column);
        let pane = Rc::new(ListsPane {
            widget,
            split,
            column,
            head,
            body,
            list,
            lines: gio::ListStore::new::<BoxedAnyObject>(),
            foot,
            lister: lister.clone(),
            open: Cell::default(),
            asked: RefCell::default(),
            folds: RefCell::default(),
            settling: Rc::default(),
            drawn: RefCell::default(),
            held: RefCell::default(),
            pulling: Cell::default(),
            return_to: glib::WeakRef::new(),
            composer: composer.clone().upcast(),
            reworder: Reworder::new(lister),
            reworded: RefCell::default(),
            reselect: Cell::default(),
            moving: RefCell::default(),
        });
        pane.wire();
        pane
    }

    fn wire(self: &Rc<Self>) {
        let pane = Rc::downgrade(self);
        self.list
            .bind_model(Some(&self.lines), move |item| line_row(&pane, item));
        let pane = Rc::downgrade(self);
        self.split.connect_collapsed_notify(move |split| {
            if let Some(pane) = pane.upgrade() {
                pane.collapsed(split.is_collapsed());
            }
        });
        let pane = Rc::downgrade(self);
        self.split.connect_show_sidebar_notify(move |split| {
            if let Some(pane) = pane.upgrade() {
                pane.sidebar_shown(split.shows_sidebar());
            }
        });
        let pane = Rc::downgrade(self);
        self.widget.connect_unmap(move |_| {
            if let Some(pane) = pane.upgrade() {
                pane.change_open(PaneOpen::chat_left);
            }
        });
        let pane = Rc::downgrade(self);
        self.widget.connect_map(move |_| {
            if let Some(pane) = pane.upgrade() {
                pane.laid_out_after_paint();
            }
        });
        let pane = Rc::downgrade(self);
        self.head.hide.connect_clicked(move |_| {
            if let Some(pane) = pane.upgrade() {
                pane.choose(false);
            }
        });
        let pane = Rc::downgrade(self);
        self.foot.add.connect_entry_activated(move |_| {
            if let Some(pane) = pane.upgrade() {
                pane.add_item();
            }
        });
        let pane = Rc::downgrade(self);
        list_more::menu_on(&self.head.more, move || pane.upgrade()?.list_menu());
        self.wire_picks();
    }

    /// The picker's folders come from its header function; a list picked is the one shown.
    fn wire_picks(self: &Rc<Self>) {
        let picked = self.head.picked.clone();
        self.head.picks.set_header_func(move |row, before| {
            let picked = picked.borrow();
            let folder_of = |row: &gtk::ListBoxRow| {
                let at = usize::try_from(row.index()).ok()?;
                picked.get(at).map(|pick| pick.folder.clone())
            };
            let header = pick_header(folder_of(row).flatten(), before.and_then(folder_of));
            row.set_header(header.as_ref());
        });
        let pane = Rc::downgrade(self);
        self.head.picks.connect_row_activated(move |_, row| {
            if let Some(pane) = pane.upgrade() {
                pane.pick(row.index());
            }
        });
    }

    /// The owner's last open or close, read at start.
    pub fn keep_open(&self, open: bool) {
        self.open.set(PaneOpen::new(open));
        self.split.set_show_sidebar(open);
    }

    pub fn shown(&self) -> bool {
        self.open.get().shown()
    }

    /// A card's open (§2.6), or Ctrl+Shift+L from another page: the pane shows, at `list_id`
    /// when one is given, saving a rewording going on first. By a key, focus moves to the picker.
    pub fn open_at(self: &Rc<Self>, list_id: Option<&str>, by_key: bool) {
        if let Some(id) = list_id {
            self.reworder.save_going();
            self.asked.replace(Some(id.to_owned()));
        }
        if !self.shown() {
            self.remember_focus();
        }
        self.choose(true);
        if by_key {
            self.focus_picker();
        }
    }

    /// The owner closed the pane from outside it: `win.lists` set to false.
    pub fn close(&self) {
        self.choose(false);
    }

    /// The toggle, and Ctrl+Shift+L on Chat: the pane as it is not shown now.
    pub fn toggle(self: &Rc<Self>, by_key: bool) {
        if self.shown() {
            return self.choose(false);
        }
        self.open_at(None, by_key);
    }

    /// The owner opened or closed the pane: the split follows, and the session keeps the choice.
    /// A close gives the focus back to the chat.
    fn choose(&self, open: bool) {
        let mut state = self.open.get();
        state.choose(open);
        self.open.set(state);
        if !open {
            self.give_focus_back();
        }
        self.split.set_show_sidebar(state.shown());
        let shown = state.shown();
        act(
            &self.lister,
            ListAct::Pane {
                shown,
                chosen: Some(open),
            },
        );
    }

    /// The chat narrowed or widened past the breakpoint: narrow, the pane hides; wide, it shows
    /// as the owner last chose. `pin-sidebar` keeps the split from doing either itself.
    fn collapsed(self: &Rc<Self>, collapsed: bool) {
        let mut state = self.open.get();
        state.collapse(collapsed);
        self.open.set(state);
        if !state.shown() && self.has_focus() {
            self.give_focus_back_later();
        }
        self.split.set_show_sidebar(state.shown());
        let shown = state.shown();
        act(
            &self.lister,
            ListAct::Pane {
                shown,
                chosen: None,
            },
        );
        self.pull_shown();
    }

    /// The split showed or hid the pane. As asked, nothing changes; by Escape or a click on the
    /// narrow chat, it is the owner's close. The split has moved the focus out of the pane by
    /// then, to the chat's first control unless it saw where it was; it goes where it was.
    fn sidebar_shown(&self, shown: bool) {
        let mut state = self.open.get();
        if !state.widget_shown(shown) {
            return;
        }
        self.open.set(state);
        if let Some(root) = self.column.root().filter(|_| !shown) {
            self.return_focus(&root);
        }
        let chosen = Some(state.chosen());
        act(&self.lister, ListAct::Pane { shown, chosen });
    }

    /// Notes where the focus is, outside the pane, for the pane's close to give it back.
    fn remember_focus(&self) {
        let focus = self.column.root().and_then(|root| root.focus());
        if let Some(focus) = focus.filter(|focus| !focus.is_ancestor(&self.column)) {
            self.return_to.set(Some(&focus));
        }
    }

    /// Whether the focus is in the pane.
    fn has_focus(&self) -> bool {
        let focus = self.column.root().and_then(|root| root.focus());
        focus.is_some_and(|focus| focus.is_ancestor(&self.column))
    }

    /// Focus in the pane, about to hide, goes back to where it was when the pane was last opened,
    /// while that is on screen, and else to the composer: never to the split's first control,
    /// the conversation's oldest message, which would scroll the conversation to its top.
    fn give_focus_back(&self) {
        if self.has_focus() {
            self.focus_into_chat();
        }
    }

    /// The chat narrowed with focus in the pane. The breakpoint bin hides the chat for a frame as
    /// it narrows, so nothing in it takes focus now, and then gives the focus back to the pane,
    /// which is hidden by then. Two frames on, the chat is back on screen: the focus goes where
    /// `give_focus_back` sends it.
    fn give_focus_back_later(self: &Rc<Self>) {
        let frames = Cell::new(0);
        let pane = Rc::downgrade(self);
        self.widget.add_tick_callback(move |_, _| {
            frames.set(frames.get() + 1);
            if frames.get() < 2 {
                return glib::ControlFlow::Continue;
            }
            if let Some(pane) = pane.upgrade().filter(|pane| !pane.shown()) {
                pane.focus_into_chat();
            }
            glib::ControlFlow::Break
        });
    }

    /// The focus goes where it was when the pane was last opened, while that is on screen, and
    /// else to the composer. The window's focus is cleared first: GTK moves focus off a hidden
    /// widget a frame later.
    fn focus_into_chat(&self) {
        let Some(root) = self.column.root() else {
            return;
        };
        let back = self.return_to.upgrade().filter(|back| back.is_mapped());
        let back = back.unwrap_or_else(|| self.composer.clone());
        root.set_focus(None::<&gtk::Widget>);
        back.grab_focus();
    }

    /// Focus goes to where it was when the pane was last opened, while that is on screen. The
    /// window's focus is cleared first: GTK moves focus off a hidden widget a frame later.
    fn return_focus(&self, root: &gtk::Root) {
        let Some(back) = self.return_to.upgrade().filter(|back| back.is_mapped()) else {
            return;
        };
        root.set_focus(None::<&gtk::Widget>);
        back.grab_focus();
    }

    /// Changes the open state by `change`, with nothing else to do.
    fn change_open(&self, change: fn(&mut PaneOpen)) {
        let mut state = self.open.get();
        change(&mut state);
        self.open.set(state);
    }

    /// The chat came on screen: once its first frame is drawn, its breakpoint is current.
    fn laid_out_after_paint(self: &Rc<Self>) {
        let Some(clock) = self.widget.frame_clock() else {
            return self.change_open(PaneOpen::chat_laid_out);
        };
        let once: Rc<Cell<Option<glib::SignalHandlerId>>> = Rc::default();
        let (pane, handler) = (Rc::downgrade(self), once.clone());
        let id = clock.connect_after_paint(move |clock| {
            if let Some(id) = handler.take() {
                clock.disconnect(id);
            }
            if let Some(pane) = pane.upgrade() {
                pane.change_open(PaneOpen::chat_laid_out);
            }
        });
        once.set(Some(id));
    }

    /// Opening by a key moves focus to the picker, once the pane is on screen.
    fn focus_picker(self: &Rc<Self>) {
        if self.head.picker.is_mapped() {
            return self.take_focus();
        }
        let waiting: Rc<Cell<Option<glib::SignalHandlerId>>> = Rc::default();
        let (pane, handler) = (Rc::downgrade(self), waiting.clone());
        let id = self.head.picker.connect_map(move |picker| {
            if let Some(id) = handler.take() {
                picker.disconnect(id);
            }
            if let Some(pane) = pane.upgrade().filter(|pane| pane.shown()) {
                pane.take_focus();
            }
        });
        waiting.set(Some(id));
    }

    /// The picker, or Hide lists while there is nothing to pick.
    fn take_focus(&self) {
        if !self.head.picker.grab_focus() {
            self.head.hide.grab_focus();
        }
    }

    /// Draws the pane as `library` holds the lists, changing only what changed, an item being
    /// reworded as its entries, and pulls the list it shows while it is shown. A rewording whose
    /// item is no longer drawn ends, with nothing sent.
    pub fn show(self: &Rc<Self>, library: &Library) {
        let reworded = self.reworder.row_id();
        let now = {
            let settling = self.settling.borrow().rows();
            let asked = self.asked.borrow();
            let drawn = drawing(library, asked.as_deref(), &self.folds.borrow(), &settling);
            rewording(drawn, reworded.as_deref())
        };
        let lines = lines_of(&now).map(|(_, lines)| lines);
        let gone = |id: &String| !lines.is_some_and(|lines| rewords(lines, id));
        if reworded.as_ref().is_some_and(gone) {
            self.reworder.dropped();
        }
        let held = match &now {
            PaneDrawing::List(list) => library.shown(&list.list_id),
            PaneDrawing::Waiting | PaneDrawing::NoLists => None,
        };
        self.held.replace(held);
        if self.drawn.borrow().as_ref() != Some(&now) {
            let was = self.drawn.take();
            self.draw(was.as_ref(), &now);
            self.drawn.replace(Some(now));
        }
        self.pull_shown();
    }

    fn draw(&self, was: Option<&PaneDrawing>, now: &PaneDrawing) {
        let old = was.and_then(lines_of);
        let PaneDrawing::List(list) = now else {
            self.head.draw_none();
            self.draw_lines(old, None);
            self.foot.draw(false, None);
            let page = match now {
                PaneDrawing::NoLists => "none",
                _ => "waiting",
            };
            return self.body.set_visible_child_name(page);
        };
        self.head.draw(list);
        self.draw_lines(old, lines_of(now));
        let page = match list.body {
            PaneBody::Waiting => "waiting",
            PaneBody::Removed => "removed",
            PaneBody::Lines(_) => "list",
        };
        self.body.set_visible_child_name(page);
        let lines = matches!(list.body, PaneBody::Lines(_));
        self.foot.draw(lines, list.refusal.as_deref());
        for said in news(was, now, self.column.is_mapped()) {
            self.column
                .announce(said, gtk::AccessibleAnnouncementPriority::Medium);
        }
    }

    /// Turns the list's rows from `old` into `new` with one splice, or all of them for another
    /// list. Focus on a row the splice took goes to the row now in its place, else the row
    /// before, else Add an item; the selected item stays selected.
    fn draw_lines(&self, old: Option<(&str, &[Line])>, new: Option<(&str, &[Line])>) {
        let change = match (old, new) {
            (Some((was, old)), Some((id, new))) if was == id => splice(old, new),
            (_, new) => Some(Splice {
                at: 0,
                removed: usize::try_from(self.lines.n_items()).unwrap_or(usize::MAX),
                added: new.map(|(_, lines)| lines.to_vec()).unwrap_or_default(),
            }),
        };
        let Some(change) = change else {
            return;
        };
        let (focused, selected) = (focused_at(&self.list), self.selected_line());
        let at_entry = focused.and_then(|at| self.list.row_at_index(at));
        let typed = at_entry.is_some_and(|row| row.is::<adw::EntryRow>());
        let typing = self.reworder.current();
        let typing = typing.as_ref().and_then(|r| r.focused()).cloned();
        let list_id = new.map_or("", |(id, _)| id);
        let (at, removed) = (index(change.at), index(change.removed));
        let added: Vec<BoxedAnyObject> = change
            .added
            .into_iter()
            .map(|line| {
                BoxedAnyObject::new(PaneLine {
                    list_id: list_id.to_owned(),
                    line,
                })
            })
            .collect();
        self.lines.splice(
            u32::try_from(at).unwrap_or(0),
            u32::try_from(removed).unwrap_or(0),
            &added,
        );
        let spliced = focused.filter(|was| (at..at + removed).contains(was));
        let in_list = |entry: &adw::EntryRow| entry.parent().is_some_and(|p| p == self.list);
        match typing.filter(in_list).filter(|_| spliced.is_some()) {
            Some(entry) => self.keep_typing(&entry),
            None if typed => self.refocus(spliced, self.reworded.borrow().as_deref()),
            None => self.refocus(spliced, self.moving.borrow().as_deref()),
        }
        self.reselect(selected);
    }

    /// The entry the owner was typing in, drawn again by a splice, takes the focus back with what
    /// is typed and its cursor as they were. The window's focus is cleared first, as `refocus`
    /// says.
    fn keep_typing(&self, entry: &adw::EntryRow) {
        if let Some(root) = self.list.root() {
            root.set_focus(None::<&gtk::Widget>);
        }
        entry.grab_focus_without_selecting();
    }

    /// Focus on a row a splice took, at `was`, goes to the row of the `item` whose rewording
    /// ended, or of the line a move gives it to, else the row now in its place, else the row
    /// before, else Add an item.
    fn refocus(&self, was: Option<i32>, item: Option<&str>) {
        let Some(was) = was else {
            return;
        };
        if let Some(root) = self.list.root() {
            root.set_focus(None::<&gtk::Widget>);
        }
        let row = item
            .and_then(|id| self.index_of(id))
            .and_then(|at| self.list.row_at_index(at))
            .or_else(|| self.list.row_at_index(was))
            .or_else(|| self.list.row_at_index(was - 1));
        match row {
            Some(row) => row.grab_focus(),
            None => self.foot.add.grab_focus(),
        };
    }

    /// The line of the selected row, by its id.
    fn selected_line(&self) -> Option<String> {
        let row = self.list.selected_row()?;
        self.line_at(row.index()).map(|line| line.id())
    }

    /// Selects the row drawing `id` again when a splice took the selected row, or the item whose
    /// rewording began while it was selected, once its row is back.
    fn reselect(&self, id: Option<String>) {
        let reworded = self.reworded.borrow().clone();
        let ended = reworded.filter(|id| self.reworder.row_id().as_ref() != Some(id));
        let back = ended.filter(|_| self.reselect.get());
        if back.is_some() {
            self.reselect.set(false);
        }
        let Some(id) = id.or(back).filter(|_| self.list.selected_row().is_none()) else {
            return;
        };
        if let Some(row) = self.index_of(&id).and_then(|at| self.list.row_at_index(at)) {
            self.list.select_row(Some(&row));
        }
    }

    /// The place of the line `id`.
    fn index_of(&self, id: &str) -> Option<i32> {
        let count = i32::try_from(self.lines.n_items()).unwrap_or(0);
        (0..count).find(|at| self.line_at(*at).is_some_and(|line| line.id() == id))
    }

    fn line_at(&self, at: i32) -> Option<Line> {
        let item = self.lines.item(u32::try_from(at).ok()?)?;
        let boxed = item.downcast::<BoxedAnyObject>().ok()?;
        let line = boxed.borrow::<PaneLine>().line.clone();
        Some(line)
    }

    /// Asks the session, at the next idle, to pull the list the pane shows while it is shown: it
    /// pulls only one the app does not hold.
    fn pull_shown(self: &Rc<Self>) {
        if !self.shown() || self.pulling.get() {
            return;
        }
        let drawn = self.drawn.borrow();
        let Some(PaneDrawing::List(list)) = drawn.as_ref() else {
            return;
        };
        let list_id = list.list_id.clone();
        self.pulling.set(true);
        let pane = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            let Some(pane) = pane.upgrade() else {
                return;
            };
            pane.pulling.set(false);
            act(&pane.lister, ListAct::Drawn(vec![list_id]));
        });
    }

    /// A list picked: the pane shows it.
    fn pick(&self, at: i32) {
        let at = usize::try_from(at).ok();
        let id = at.and_then(|at| Some(self.head.picked.borrow().get(at)?.list_id.clone()));
        self.head.picker.popdown();
        if let Some(id) = id {
            self.asked.replace(Some(id));
            act(&self.lister, ListAct::Redraw);
        }
    }

    /// Return in Add an item: the words go to the selected item's section, or the first group,
    /// and the field stays, empty, for the next item. Words that trim to nothing add nothing.
    fn add_item(&self) {
        let text = self.foot.add.text().trim().to_owned();
        if text.is_empty() {
            return;
        }
        let selected = self
            .list
            .selected_row()
            .and_then(|row| self.line_at(row.index()));
        let selected = selected.and_then(|line| line.editable().map(|row| row.row_id.clone()));
        let place = {
            let held = self.held.borrow();
            let Some(list) = held.as_ref() else {
                return;
            };
            (list.list_id.clone(), add_place(list, selected.as_deref()))
        };
        self.foot.add.set_text("");
        let (list_id, place) = place;
        let (section_id, after_id) = Place::fields(place);
        let op = ListOp::Add {
            text,
            note: None,
            section_id,
            after_id,
        };
        act(&self.lister, ListAct::Edit { list_id, op });
    }

    fn row_of(self: &Rc<Self>, line: &PaneLine) -> gtk::ListBoxRow {
        let row: adw::PreferencesRow = match &line.line {
            Line::Heading { row, folded } => self.heading_row(&line.list_id, row, *folded).upcast(),
            Line::Item(row) | Line::Done(row) => self.item_row(&line.list_id, row).upcast(),
            Line::DoneHeading { count, open } => {
                self.done_row(&line.list_id, *count, *open).upcast()
            }
            Line::RemovedHeading => plain_row(RECENTLY_REMOVED).upcast(),
            Line::Removed(row) => {
                let removed = plain_row(&row.text);
                removed.add_css_class("dimmed");
                removed.upcast()
            }
            Line::Words(row_id) => self.entry_row(self.reworder.words_of(row_id)),
            Line::Note(row_id) => self.entry_row(self.reworder.note_of(row_id)),
        };
        row.upcast()
    }

    /// An item with its note, whose tick the session sends; a double-click, Return or F2 rewords
    /// its words and note; a right-click, a touch long press, Menu or Shift+F10 opens its menu
    /// (§3.6), and Alt+Up and Alt+Down move it (§6.1). A row still being sent is insensitive, so
    /// it takes none of these.
    fn item_row(self: &Rc<Self>, list_id: &str, row: &ListRow) -> adw::ActionRow {
        let item = list_rows::item_row(row, true, list_id, &self.lister, &self.settling);
        let (pane, list, words) = (Rc::downgrade(self), list_id.to_owned(), row.clone());
        rewords_on(&item, move || {
            if let Some(pane) = pane.upgrade() {
                pane.reword(&list, &words);
            }
        });
        let (pane, list, id) = (Rc::downgrade(self), list_id.to_owned(), row.row_id.clone());
        list_menu::menu_on(&item, move |item, at| {
            if let Some(pane) = pane.upgrade() {
                pane.item_menu(item, &list, &id, at);
            }
        });
        let (pane, list, id) = (Rc::downgrade(self), list_id.to_owned(), row.row_id.clone());
        moves_on(&item, move |up| {
            if let Some(pane) = pane.upgrade() {
                pane.step(&list, &id, up);
            }
        });
        item
    }

    /// The menu of the item `row_id` of `list_id`, over `item`, pointing at (x, y) after a
    /// pointer or a touch: Move to list with the other lists, and Move up and Move down where the
    /// item steps that way. A rewording going on is saved first, as a click on another row saves
    /// it.
    fn item_menu(
        self: &Rc<Self>,
        item: &adw::ActionRow,
        list_id: &str,
        row_id: &str,
        at: Option<(f64, f64)>,
    ) {
        self.reworder.save_going();
        let steps = self.steps_of(list_id, row_id).unwrap_or_default();
        let offers = ItemOffers {
            others: self.others(),
            up: steps.up.is_some(),
            down: steps.down.is_some(),
        };
        let (pane, list, id) = (Rc::downgrade(self), list_id.to_owned(), row_id.to_owned());
        let to_list = move |to: &str| {
            if let Some(pane) = pane.upgrade() {
                pane.move_to(&list, &id, to);
            }
        };
        let (pane, list, id) = (Rc::downgrade(self), list_id.to_owned(), row_id.to_owned());
        let step = move |up| {
            if let Some(pane) = pane.upgrade() {
                pane.step(&list, &id, up);
            }
        };
        let actions = list_menu::item_actions(&offers, to_list, step);
        list_menu::pop_up(item, &list_menu::item_menu(&offers.others), &actions, at);
    }

    /// Move up and Move down for the item `row_id`, while the pane holds `list_id`.
    fn steps_of(&self, list_id: &str, row_id: &str) -> Option<Steps> {
        let held = self.held.borrow();
        let list = held.as_ref().filter(|list| list.list_id == list_id)?;
        Some(panel::steps(list, row_id))
    }

    /// Where the item `row_id` of `list_id` steps up or down, and the line that takes the focus
    /// once it has: the item, or the heading of the folded section the step takes it into.
    fn step_of(&self, list_id: &str, row_id: &str, up: bool) -> Option<(Place, String)> {
        let held = self.held.borrow();
        let list = held.as_ref().filter(|list| list.list_id == list_id)?;
        let steps = panel::steps(list, row_id);
        let place = if up { steps.up } else { steps.down }?;
        let view = self.folds.borrow().view(list_id, &BTreeSet::new());
        let focus = panel::hidden_under(list, &view, &place).unwrap_or_else(|| row_id.to_owned());
        Some((place, focus))
    }

    /// The lists other than the one shown, by folder: where an item can move to.
    fn others(&self) -> Folders {
        match self.drawn.borrow().as_ref() {
            Some(PaneDrawing::List(list)) => list.folders.without(&list.list_id),
            _ => Folders::default(),
        }
    }

    /// Moves the item `row_id` of `list_id` past the line above or below it, as Move up and Move
    /// down, Alt+Up and Alt+Down do, drawn at once. The focus goes to the item in its new place,
    /// or, where the step takes it into a folded section, to that section's heading, so a held
    /// Alt+Up stops there. Nothing where no op can say the step.
    fn step(&self, list_id: &str, row_id: &str, up: bool) {
        let Some((place, focus)) = self.step_of(list_id, row_id, up) else {
            return;
        };
        let (section_id, after_id) = Place::fields(Some(place));
        let op = ListOp::Move {
            row_id: row_id.to_owned(),
            section_id,
            after_id,
            to_list_id: None,
        };
        self.moving.replace(Some(focus));
        let list_id = list_id.to_owned();
        act(&self.lister, ListAct::Edit { list_id, op });
        self.moving.replace(None);
    }

    /// Moves the item `row_id` of `list_id` to the list `to`, where the engine puts it at the end
    /// of the first group. It leaves this list at once.
    fn move_to(&self, list_id: &str, row_id: &str, to: &str) {
        let op = ListOp::Move {
            row_id: row_id.to_owned(),
            section_id: None,
            after_id: None,
            to_list_id: Some(to.to_owned()),
        };
        let list_id = list_id.to_owned();
        act(&self.lister, ListAct::Edit { list_id, op });
    }

    /// The list's menu as More opens it, for the list drawn: what it offers, and what each item
    /// does to that list. A rewording going on is saved first, as a click on another row saves
    /// it; the draw that saving makes leaves the menu opening. None while no list is drawn, or the
    /// one drawn is removed.
    fn list_menu(self: &Rc<Self>) -> Option<(ListOffers, ListActs)> {
        self.head.menu_of.replace(None);
        self.reworder.save_going();
        let (offers, list_id) = match self.drawn.borrow().as_ref() {
            Some(PaneDrawing::List(list)) => (list_offers(list)?, list.list_id.clone()),
            _ => return None,
        };
        let made = Some((list_id.clone(), offers.clone()));
        self.head.menu_of.replace(made);
        let (pane, list, offered) = (Rc::downgrade(self), list_id.clone(), offers.clone());
        let folder = move |picked: &str| {
            if let Some(pane) = pane.upgrade() {
                pane.to_folder(&list, &offered, picked);
            }
        };
        let (pane, list) = (Rc::downgrade(self), list_id.clone());
        let copy = move || {
            if let Some(pane) = pane.upgrade() {
                pane.copy(&list);
            }
        };
        let (pane, list) = (Rc::downgrade(self), list_id);
        let clear_done = move || {
            if let Some(pane) = pane.upgrade() {
                pane.clear_done(&list);
            }
        };
        let acts = ListActs {
            folder: Rc::new(folder),
            copy: Rc::new(copy),
            clear_done: Rc::new(clear_done),
        };
        Some((offers, acts))
    }

    /// Closes More's menu, which a folder picked with Space leaves open, then moves `list_id` to
    /// `picked`, empty for no folder, as `offers` says: nothing for the folder it will be in once
    /// its edits land. The head shows the new folder when the index comes back.
    fn to_folder(&self, list_id: &str, offers: &ListOffers, picked: &str) {
        self.head.more.popdown();
        if let Some(op) = folder_op(offers, picked) {
            let list_id = list_id.to_owned();
            act(&self.lister, ListAct::Edit { list_id, op });
        }
    }

    /// Puts `list_id` on the clipboard as text, the whole list as drawn with its edits waiting,
    /// with the app's "Copied" toast.
    fn copy(&self, list_id: &str) {
        let held = self.held.borrow();
        match held.as_ref().filter(|list| list.list_id == list_id) {
            Some(list) => copy_text(&self.column, &list.markdown()),
            None => glib::g_warning!("fermix", "Copy as text found its list no longer drawn"),
        }
    }

    /// Clears the done items of `list_id`, drawn at once.
    fn clear_done(&self, list_id: &str) {
        let list_id = list_id.to_owned();
        let op = ListOp::ClearDone;
        act(&self.lister, ListAct::Edit { list_id, op });
    }

    /// Rewords `row`, keeping its selection for when its row is back. A rewording going on is
    /// saved first.
    fn reword(&self, list_id: &str, row: &ListRow) {
        self.reworder.save_going();
        let selected = self.selected_line().is_some_and(|id| id == row.row_id);
        self.reselect.set(selected);
        self.reworded.replace(Some(row.row_id.clone()));
        self.reworder.start(list_id, row, true);
    }

    /// An entry of the item being reworded, kept while it is. A line drawn for no rewording, or
    /// for one whose entry is still in a list, is a mistake: it draws an empty row, logged.
    fn entry_row(&self, entry: Option<adw::EntryRow>) -> adw::PreferencesRow {
        match entry.filter(|entry| entry.parent().is_none()) {
            Some(entry) => entry.upcast(),
            None => {
                glib::g_warning!("fermix", "the Lists pane drew an entry for no rewording");
                plain_row("").upcast()
            }
        }
    }

    /// A section's heading in `list_id`: a single click, Return or Space folds or opens it. A
    /// rewording going on is saved first, as a click on another row saves it.
    fn heading_row(self: &Rc<Self>, list_id: &str, row: &ListRow, folded: bool) -> adw::ActionRow {
        let heading = fold_row(&row.text, &row.text, !folded);
        let pane = Rc::downgrade(self);
        let (list_id, id) = (list_id.to_owned(), row.row_id.clone());
        on_click_or_key(&heading, move |_| {
            if let Some(pane) = pane.upgrade() {
                pane.reworder.save_going();
                pane.folds.borrow_mut().fold(&list_id, &id);
                act(&pane.lister, ListAct::Redraw);
            }
        });
        heading
    }

    /// "Done · 2" in `list_id`: a single click, Return or Space opens or closes the done items. A
    /// rewording going on is saved first.
    fn done_row(self: &Rc<Self>, list_id: &str, count: usize, open: bool) -> adw::ActionRow {
        let spoken = words::spoken_done_heading(count);
        let done = fold_row(&words::done_heading(count), &spoken, open);
        let (pane, list_id) = (Rc::downgrade(self), list_id.to_owned());
        on_click_or_key(&done, move |_| {
            if let Some(pane) = pane.upgrade() {
                pane.reworder.save_going();
                pane.folds.borrow_mut().open_done(&list_id);
                act(&pane.lister, ListAct::Redraw);
            }
        });
        done
    }
}

impl Head {
    fn new() -> Head {
        let title = gtk::Label::new(None);
        title.set_use_markup(false);
        title.set_text(LISTS);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.add_css_class("heading");
        let picks = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .propagate_natural_width(true)
            .max_content_height(PICKS_HEIGHT)
            .child(&picks)
            .build();
        let popover = gtk::Popover::builder().child(&scroller).build();
        let picker = gtk::MenuButton::builder()
            .popover(&popover)
            .always_show_arrow(true)
            .hexpand(true)
            .halign(gtk::Align::Start)
            .css_classes(["flat"])
            .sensitive(false)
            .build();
        picker.set_child(Some(&title));
        let hide = gtk::Button::from_icon_name(PANE_MARK);
        hide.add_css_class("flat");
        hide.set_tooltip_text(Some(HIDE_LISTS));
        hide.update_property(&[gtk::accessible::Property::Label(HIDE_LISTS)]);
        let counts = gtk::Label::new(None);
        counts.set_use_markup(false);
        counts.set_xalign(0.0);
        counts.set_ellipsize(gtk::pango::EllipsizeMode::End);
        counts.add_css_class("dimmed");
        counts.set_margin_start(18);
        counts.set_margin_end(18);
        counts.set_visible(false);
        Head {
            picker,
            title,
            picks,
            picked: Rc::default(),
            picks_of: RefCell::default(),
            more: list_more::more_button(),
            menu_of: RefCell::default(),
            hide,
            counts,
        }
    }

    /// Before the index answers, and with no lists: "Lists", with nothing to pick.
    fn draw_none(&self) {
        self.title.set_text(LISTS);
        self.picker.set_sensitive(false);
        self.show_more(false);
        self.counts.set_visible(false);
        self.picked.borrow_mut().clear();
        self.picks_of.replace(None);
        self.picks.remove_all();
    }

    fn draw(&self, list: &PaneList) {
        self.title.set_text(&list.title);
        self.picker.set_sensitive(true);
        let offers = list_offers(list);
        self.close_stale_menu(&list.list_id, offers.as_ref());
        self.show_more(offers.is_some());
        self.counts.set_text(&list.head);
        let spoken = gtk::accessible::Property::Label(&list.spoken_head);
        self.counts.update_property(&[spoken]);
        self.counts.set_visible(true);
        let picks_of = Some((list.folders.clone(), list.list_id.clone()));
        if *self.picks_of.borrow() != picks_of {
            self.draw_picks(&list.folders, &list.list_id);
            self.picks_of.replace(picks_of);
        }
    }

    /// Closes More's menu while it shows when `list_id` or its `offers` are not what the menu was
    /// made for, as an item's menu closes when its row goes: its items would act on what was drawn.
    fn close_stale_menu(&self, list_id: &str, offers: Option<&ListOffers>) {
        let stale = self
            .menu_of
            .borrow()
            .as_ref()
            .is_some_and(|(made_for, made)| made_for != list_id || Some(made) != offers);
        if stale && self.more.is_active() {
            self.more.popdown();
        }
    }

    /// Shows More, or hides it. Hiding it while it holds the focus gives the focus to Hide lists,
    /// the window's focus cleared first, as GTK would move it out of the pane a frame later.
    fn show_more(&self, shown: bool) {
        let root = self.more.root();
        let focus = root.as_ref().and_then(|root| root.focus());
        let more: &gtk::Widget = self.more.upcast_ref();
        let held = focus.is_some_and(|focus| focus == *more || focus.is_ancestor(more));
        if !shown && held {
            if let Some(root) = root {
                root.set_focus(None::<&gtk::Widget>);
            }
            self.hide.grab_focus();
        }
        self.more.set_visible(shown);
    }

    /// The picker's lists: each folder's in the index's order, then those in no folder, each
    /// with its open count and a check on the one shown.
    fn draw_picks(&self, folders: &Folders, shown: &str) {
        let in_folders = folders
            .folders
            .iter()
            .flat_map(|folder| folder.lists.iter());
        let entries: Vec<&ListEntry> = in_folders.chain(folders.loose.iter()).collect();
        let picks = entries.iter().map(|entry| Pick {
            list_id: entry.list_id.clone(),
            folder: entry.folder.clone(),
        });
        self.picked.replace(picks.collect());
        self.picks.remove_all();
        for entry in entries {
            self.picks.append(&pick_row(entry, entry.list_id == shown));
        }
    }
}

impl Foot {
    fn new() -> Foot {
        let refusal = gtk::Label::new(None);
        refusal.set_use_markup(false);
        refusal.set_wrap(true);
        refusal.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        refusal.set_xalign(0.0);
        refusal.set_visible(false);
        let add = adw::EntryRow::new();
        add.set_use_markup(false);
        add.set_title(ADD_ITEM);
        let field = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        field.append(&add);
        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(12)
            .visible(false)
            .build();
        widget.append(&refusal);
        widget.append(&field);
        Foot {
            widget,
            refusal,
            add,
        }
    }

    /// Shown while a list's lines are drawn, with the sentence for an edit refused.
    fn draw(&self, lines: bool, refusal: Option<&str>) {
        self.widget.set_visible(lines);
        self.refusal.set_text(refusal.unwrap_or_default());
        self.refusal.set_visible(refusal.is_some());
    }
}

/// The row a line of the list's model draws.
fn line_row(pane: &Weak<ListsPane>, item: &glib::Object) -> gtk::Widget {
    let boxed = item.downcast_ref::<BoxedAnyObject>();
    let (Some(pane), Some(boxed)) = (pane.upgrade(), boxed) else {
        glib::g_warning!("fermix", "a line of the Lists pane had no pane to draw it");
        return gtk::ListBoxRow::new().upcast();
    };
    let line = boxed.borrow::<PaneLine>();
    pane.row_of(&line).upcast()
}

/// A heading that folds what is under it: its words, with `pan-down` while open and `pan-end`
/// while folded, and its expanded state told to a screen reader.
fn fold_row(text: &str, spoken: &str, expanded: bool) -> adw::ActionRow {
    let row = plain_row(text);
    let mark = match expanded {
        true => "pan-down-symbolic",
        false => "pan-end-symbolic",
    };
    let mark = gtk::Image::from_icon_name(mark);
    mark.set_accessible_role(gtk::AccessibleRole::Presentation);
    row.add_suffix(&mark);
    let widget = row.upcast_ref::<gtk::Widget>();
    widget.update_property(&[gtk::accessible::Property::Label(spoken)]);
    widget.update_state(&[gtk::accessible::State::Expanded(Some(expanded))]);
    row
}

fn plain_row(text: &str) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_use_markup(false);
    row.set_title(text);
    row
}

/// A list in the picker: its title, its open count, and a check while it is the one shown.
fn pick_row(entry: &ListEntry, shown: bool) -> adw::ActionRow {
    let row = plain_row(&entry.title);
    row.set_activatable(true);
    let mark = gtk::Image::from_icon_name("object-select-symbolic");
    mark.set_opacity(if shown { 1.0 } else { 0.0 });
    mark.set_accessible_role(gtk::AccessibleRole::Presentation);
    row.add_prefix(&mark);
    let open = usize::try_from(entry.open).unwrap_or(usize::MAX);
    let count = gtk::Label::new(None);
    count.set_use_markup(false);
    count.set_text(&words::open(open));
    count.add_css_class("dimmed");
    row.add_suffix(&count);
    let widget = row.upcast_ref::<gtk::Widget>();
    let spoken = words::list_label(&entry.title, open);
    widget.update_property(&[gtk::accessible::Property::Label(&spoken)]);
    widget.update_state(&[gtk::accessible::State::Selected(Some(shown))]);
    row
}

/// A folder's heading over its first list, and a separator over the first list in no folder
/// after the folders.
fn pick_header(folder: Option<String>, before: Option<Option<String>>) -> Option<gtk::Widget> {
    match (folder, before) {
        (Some(name), Some(Some(previous))) if name == previous => None,
        (Some(name), _) => {
            let heading = gtk::Label::new(None);
            heading.set_use_markup(false);
            heading.set_text(&name);
            heading.set_xalign(0.0);
            heading.set_margin_start(12);
            heading.set_margin_top(6);
            heading.add_css_class("heading");
            heading.add_css_class("dimmed");
            Some(heading.upcast())
        }
        (None, Some(Some(_))) => Some(gtk::Separator::new(gtk::Orientation::Horizontal).upcast()),
        (None, _) => None,
    }
}

/// What the pane says, by what it shows: before the index answers, no lists, a list waiting
/// for its pull, a removed list, and the list.
fn body_stack(list: &gtk::ListBox) -> gtk::Stack {
    let body = gtk::Stack::new();
    body.set_vexpand(true);
    let waiting = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.add_named(&waiting, Some("waiting"));
    body.add_named(&status(NO_LISTS), Some("none"));
    body.add_named(&status(LIST_REMOVED), Some("removed"));
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(list)
        .build();
    body.add_named(&scroller, Some("list"));
    body.set_visible_child_name("waiting");
    body
}

fn status(title: &str) -> adw::StatusPage {
    let page = adw::StatusPage::new();
    page.set_title(title);
    page.add_css_class("compact");
    page
}

/// The pane: a group labelled "Lists" holding the head, the list and the foot.
fn column(head: &Head, body: &gtk::Stack, foot: &Foot) -> gtk::Box {
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .accessible_role(gtk::AccessibleRole::Group)
        .build();
    column.update_property(&[gtk::accessible::Property::Label(LISTS)]);
    let bar = gtk::Box::builder()
        .spacing(6)
        .margin_start(6)
        .margin_end(6)
        .margin_top(6)
        .build();
    bar.append(&head.picker);
    bar.append(&head.more);
    bar.append(&head.hide);
    column.append(&bar);
    column.append(&head.counts);
    column.append(body);
    column.append(&foot.widget);
    column
}

/// The split at the chat's end (§3.1), in a bin that collapses it below 600sp. `pin-sidebar`
/// keeps the split from showing the pane on every widening; the pane shows what the owner chose.
fn split_around(
    chat: &impl IsA<gtk::Widget>,
    column: &gtk::Box,
) -> (adw::BreakpointBin, adw::OverlaySplitView) {
    let split = adw::OverlaySplitView::builder()
        .sidebar_position(gtk::PackType::End)
        .sidebar_width_fraction(FRACTION)
        .min_sidebar_width(MIN_WIDTH)
        .max_sidebar_width(MAX_WIDTH)
        .sidebar_width_unit(adw::LengthUnit::Sp)
        .pin_sidebar(true)
        .show_sidebar(false)
        .content(chat)
        .sidebar(column)
        .build();
    let bin = adw::BreakpointBin::builder()
        .width_request(MIN_SIZE.0)
        .height_request(MIN_SIZE.1)
        .child(&split)
        .build();
    let condition = adw::BreakpointCondition::parse(NARROW).expect("the breakpoint's fixed words");
    let narrow = adw::Breakpoint::new(condition);
    narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
    bin.add_breakpoint(narrow);
    (bin, split)
}

/// A drawing's list and lines, when it draws lines.
fn lines_of(drawing: &PaneDrawing) -> Option<(&str, &[Line])> {
    let PaneDrawing::List(list) = drawing else {
        return None;
    };
    match &list.body {
        PaneBody::Lines(lines) => Some((list.list_id.as_str(), lines.as_slice())),
        PaneBody::Waiting | PaneBody::Removed => None,
    }
}

/// The row holding the focus in `list`, by its place.
fn focused_at(list: &gtk::ListBox) -> Option<i32> {
    let focus = list.root()?.focus()?;
    let row = focus.ancestor(gtk::ListBoxRow::static_type())?;
    let row = row.downcast::<gtk::ListBoxRow>().ok()?;
    row.parent()
        .filter(|parent| parent == list.upcast_ref::<gtk::Widget>())?;
    Some(row.index())
}

fn index(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}
