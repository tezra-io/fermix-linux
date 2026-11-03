//! A reply's lists card (M85 lists §2), under its place cards: the owner's lists its turn read or
//! changed, drawn as they stand now (`fermix_client::checklists::card`). A live card of one list
//! shows its open items with "N more" and Add an item, a card of several a group per list, and a
//! folded card one button per list. A list's head, its Open list button, a group's name and a
//! folded button open the Lists pane at that list. Only stock widgets, markup off before any
//! text, and no list edit is an exported action: a tick is its check button's own closure, and
//! the card hands each edit to the session through `Lister`.

use adw::prelude::*;
use fermix_client::chat::ListsCard;
use fermix_client::chat_wire::protocol::{ListOp, ListRow};
use fermix_client::checklists::card::{drawing, news, Drawing, Head, ListGroup, Settling, Stands};
use fermix_client::checklists::cards::OneList;
use fermix_client::checklists::library::Library;
use fermix_client::checklists::words::{self, ADD_ITEM, LIST_REMOVED, OPEN_LIST, SHOW_LESS};
use gtk::glib;
use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;

use super::list_rows::{self, button_on_click_or_key, on_click_or_key, on_press, LIST_MARK};
use super::places::{CARDS_MIN_WIDTH, CARDS_WIDTH};

/// A card's Open list button, and the pane's Hide lists.
pub const PANE_MARK: &str = "sidebar-show-right-symbolic";

/// What a reply's lists card and the Lists pane ask of the session.
pub enum ListAct {
    /// A live card came on screen, or the pane shows a list: each list is pulled unless the app
    /// holds it. The page asks this while it holds its entries, so the answer draws nothing: a
    /// pull changes no card until its list arrives, and that arrival draws. A card asks once each
    /// time it comes on screen, not at every look.
    Drawn(Vec<String>),
    /// An edit of one list, from a tick or Add an item.
    Edit { list_id: String, op: ListOp },
    /// A card's or the pane's own state changed (grown, folded back, a section or Done folded or
    /// opened, a list picked, or a ticked row settled): the lists are drawn again.
    Redraw,
    /// A card opens the pane at its list. By a key (Return on its row), focus moves to the
    /// pane's picker.
    Open { list_id: String, by_key: bool },
    /// The pane was shown or hidden; `chosen` is the owner's open or close to keep, when it was
    /// one.
    Pane { shown: bool, chosen: Option<bool> },
}

/// Where the cards send what they ask: the session, handed over once it exists.
pub type Lister = Rc<OnceCell<Box<dyn Fn(ListAct)>>>;

pub fn act(lister: &Lister, asked: ListAct) {
    match lister.get() {
        Some(take) => take(asked),
        None => glib::g_warning!("fermix", "a lists card acted before the chat could take it"),
    }
}

/// The card under one reply; hidden until its row names lists.
pub struct ListsView {
    pub widget: adw::Clamp,
    column: gtk::Box,
    /// Kept across draws, so words typed into it survive a push.
    add: adw::EntryRow,
    /// The list Add an item adds to: the one the card draws.
    add_to: Rc<RefCell<String>>,
    lister: Lister,
    card: Option<ListsCard>,
    /// The one-list card holds every open item, not four.
    grown: Rc<Cell<bool>>,
    /// The rows ticked a moment ago, which keep their place while they settle.
    settling: Rc<RefCell<Settling>>,
    /// What the card drew last, so a draw that changes nothing leaves the widgets as they are.
    drawn: Option<Drawing>,
}

impl ListsView {
    pub fn new(lister: &Lister) -> ListsView {
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .width_request(CARDS_MIN_WIDTH)
            .build();
        // A clamp past its tightening threshold grows wider than its child and centres it, so a
        // card with long rows would leave the reply's edge; with none, it stays start-aligned.
        let widget = adw::Clamp::builder()
            .maximum_size(CARDS_WIDTH)
            .tightening_threshold(CARDS_WIDTH)
            .halign(gtk::Align::Start)
            .child(&column)
            .visible(false)
            .build();
        let add_to = Rc::new(RefCell::new(String::new()));
        ListsView {
            widget,
            column,
            add: add_row(lister, &add_to),
            add_to,
            lister: lister.clone(),
            card: None,
            grown: Rc::default(),
            settling: Rc::default(),
            drawn: None,
        }
    }

    /// The card the reply's row now names, drawn at the next `show`.
    pub fn set_card(&mut self, card: Option<&ListsCard>) {
        self.card = card.cloned();
    }

    /// Draws the card as `library` holds its lists, rebuilding its rows only when what it draws
    /// changed, and speaks what core's `news` says is new on it. `on_screen`: the card was on
    /// screen at the page's last look; off screen, it speaks nothing.
    pub fn show(&mut self, library: &Library, on_screen: bool) {
        let drawn = self.card.as_ref().and_then(|card| {
            let settling = self.settling.borrow().rows();
            drawing(&card.lists, card.live, library, self.grown.get(), &settling)
        });
        if drawn == self.drawn {
            return;
        }
        let focused = focused_row(&self.column, &self.add);
        clear(&self.column);
        if let Some(drawing) = &drawn {
            self.draw(drawing);
            for said in news(self.drawn.as_ref(), drawing, on_screen) {
                self.column
                    .announce(said, gtk::AccessibleAnnouncementPriority::Medium);
            }
        }
        refocus(&self.column, focused, &self.add);
        self.widget.set_visible(drawn.is_some());
        self.drawn = drawn;
    }

    /// The reply came on screen: a live card pulls the lists it draws, unless the app holds them.
    pub fn came_on_screen(&self) {
        let Some(card) = &self.card else {
            return;
        };
        let pulls: Vec<String> = card.pulls().iter().map(|l| l.list_id.clone()).collect();
        if !pulls.is_empty() {
            act(&self.lister, ListAct::Drawn(pulls));
        }
    }

    fn draw(&self, drawing: &Drawing) {
        match drawing {
            Drawing::One {
                head,
                rows,
                refusal,
            } => self.draw_one(head, rows, refusal.as_deref()),
            Drawing::Several { groups, more_lists } => self.draw_several(groups, *more_lists),
            Drawing::Folded(heads) => self.column.append(&folded_line(heads, &self.lister)),
        }
    }

    /// One list (§2.2): its head, its open items, the engine's sentence for an edit it refused,
    /// "N more" or "Show less", and Add an item.
    fn draw_one(&self, head: &Head, rows: &OneList, refusal: Option<&str>) {
        let list = card_list();
        let open = match head.stands {
            Stands::Counted { open, .. } => open,
            Stands::Waiting | Stands::Removed => 0,
        };
        list.update_property(&[gtk::accessible::Property::Label(&words::list_label(
            &head.title,
            open,
        ))]);
        list.append(&head_row(head, false, &self.lister));
        for row in &rows.rows {
            list.append(&self.item_row(&head.list_id, row));
        }
        if let Some(sentence) = refusal {
            list.append(&sentence_row(sentence));
        }
        if let Some(button) = self.more_row(rows) {
            list.append(&button);
        }
        if matches!(head.stands, Stands::Counted { .. }) {
            self.add_to.replace(head.list_id.clone());
            list.append(&self.add);
        }
        self.column.append(&list);
    }

    /// Several lists (§2.3): each group's head, with its open count, over the rows its turn
    /// touched, tick only, and the engine's sentence for an edit of it that it refused; then
    /// "and N more lists".
    fn draw_several(&self, groups: &[ListGroup], more_lists: usize) {
        let list = card_list();
        let titles: Vec<&str> = groups.iter().map(|g| g.head.title.as_str()).collect();
        list.update_property(&[gtk::accessible::Property::Label(&words::lists_label(
            &titles,
        ))]);
        for group in groups {
            list.append(&head_row(&group.head, true, &self.lister));
            for row in &group.rows {
                list.append(&self.item_row(&group.head.list_id, row));
            }
            if let Some(sentence) = &group.refusal {
                list.append(&sentence_row(sentence));
            }
        }
        if more_lists > 0 {
            list.append(&more_lists_row(more_lists));
        }
        self.column.append(&list);
    }

    /// "N more", or "Show less" on a grown card, either acting on a single click; none when every
    /// open item shows on a card that never grew.
    fn more_row(&self, rows: &OneList) -> Option<adw::ButtonRow> {
        let title = match (rows.more, rows.fewer) {
            (0, false) => return None,
            (0, true) => SHOW_LESS.to_owned(),
            (more, _) => words::more(more),
        };
        let button = adw::ButtonRow::new();
        button.set_use_markup(false);
        button.set_title(&title);
        let (grown, lister) = (self.grown.clone(), self.lister.clone());
        button_on_click_or_key(&button, move || {
            grown.set(!grown.get());
            act(&lister, ListAct::Redraw);
        });
        Some(button)
    }

    /// An item whose tick the session sends (`list_rows::item_row`), without its note.
    fn item_row(&self, list_id: &str, row: &ListRow) -> adw::ActionRow {
        list_rows::item_row(row, false, list_id, &self.lister, &self.settling)
    }
}

/// Add an item: Return adds the words to the first group of the list the card draws, and keeps
/// the field, empty, for the next item. Words that trim to nothing add nothing.
fn add_row(lister: &Lister, add_to: &Rc<RefCell<String>>) -> adw::EntryRow {
    let add = adw::EntryRow::new();
    add.set_use_markup(false);
    add.set_title(ADD_ITEM);
    let (lister, add_to) = (lister.clone(), add_to.clone());
    add.connect_entry_activated(move |add| {
        let text = add.text().trim().to_owned();
        if text.is_empty() {
            return;
        }
        add.set_text("");
        let op = ListOp::Add {
            text,
            note: None,
            section_id: None,
            after_id: None,
        };
        let list_id = add_to.borrow().clone();
        act(&lister, ListAct::Edit { list_id, op });
    });
    add
}

/// The card's list: boxed, no selection, a row activated by a double-click.
fn card_list() -> gtk::ListBox {
    gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .activate_on_single_click(false)
        .css_classes(["boxed-list"])
        .build()
}

/// A list's head: its title over its counts, or that it was removed; the block's title alone
/// while its pull is not answered. A group of a card of several counts its open items alone
/// (§2.3). Unless removed, it opens the pane at its list: a one-list card's head by its Open list
/// button too, a group's name with a go-next mark.
fn head_row(head: &Head, several: bool, lister: &Lister) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_use_markup(false);
    row.set_title(&head.title);
    let mark = gtk::Image::from_icon_name(LIST_MARK);
    mark.add_css_class("dimmed");
    mark.set_accessible_role(gtk::AccessibleRole::Presentation);
    row.add_prefix(&mark);
    match head.stands {
        Stands::Counted { open, done } => {
            let (counts, spoken) = match several {
                true => (words::open(open), words::open(open)),
                false => (words::counts(open, done), words::spoken_counts(open, done)),
            };
            row.set_subtitle(&counts);
            let described = row.upcast_ref::<gtk::Widget>();
            described.update_property(&[gtk::accessible::Property::Description(&spoken)]);
        }
        Stands::Removed => row.set_subtitle(LIST_REMOVED),
        Stands::Waiting => {}
    }
    if head.stands != Stands::Removed {
        opens_the_pane(&row, &head.list_id, several, lister);
    }
    row
}

/// A single click, Return or Space on the row opens the pane at the list, and so does a one-list
/// card's Open list button.
fn opens_the_pane(row: &adw::ActionRow, list_id: &str, several: bool, lister: &Lister) {
    let (lister, list_id) = (lister.clone(), list_id.to_owned());
    let open = move |by_key| {
        let list_id = list_id.clone();
        act(&lister, ListAct::Open { list_id, by_key });
    };
    if several {
        let next = gtk::Image::from_icon_name("go-next-symbolic");
        next.set_accessible_role(gtk::AccessibleRole::Presentation);
        row.add_suffix(&next);
        return on_click_or_key(row, open);
    }
    let button = gtk::Button::from_icon_name(PANE_MARK);
    button.add_css_class("flat");
    button.set_valign(gtk::Align::Center);
    button.set_tooltip_text(Some(OPEN_LIST));
    let spoken = button.upcast_ref::<gtk::Widget>();
    spoken.update_property(&[gtk::accessible::Property::Label(OPEN_LIST)]);
    let open = Rc::new(open);
    let pressed = open.clone();
    on_press(&button, move |by_key| pressed(by_key));
    row.add_suffix(&button);
    on_click_or_key(row, move |by_key| open(by_key));
}

/// "and N more lists", a sentence in a row that is not activatable.
fn more_lists_row(count: usize) -> gtk::ListBoxRow {
    let label = gtk::Label::new(None);
    label.set_use_markup(false);
    label.set_text(&words::more_lists(count));
    label.set_xalign(0.0);
    label.set_margin_top(8);
    label.set_margin_bottom(8);
    label.set_margin_start(12);
    label.add_css_class("caption");
    label.add_css_class("dimmed");
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_child(Some(&label));
    row
}

/// A folded card (§2.4): one button per list it drew, with the list's mark, counted from the
/// index, which opens the pane at its list; a removed list says so, as a label. Each part is one
/// line, cut short at its end when it is wider than the card, so a long title never sets the
/// chat's width; a screen reader hears it whole.
fn folded_line(heads: &[Head], lister: &Lister) -> adw::WrapBox {
    let line = adw::WrapBox::new();
    line.set_child_spacing(6);
    line.set_line_spacing(6);
    for head in heads {
        line.append(&folded_part(head, lister));
    }
    line
}

fn folded_part(head: &Head, lister: &Lister) -> gtk::Widget {
    let Stands::Counted { open, .. } = head.stands else {
        let removed = matches!(head.stands, Stands::Removed);
        let label = one_line(if removed { LIST_REMOVED } else { &head.title });
        label.add_css_class("dimmed");
        return label.upcast();
    };
    let content = adw::ButtonContent::new();
    content.set_icon_name(LIST_MARK);
    content.set_use_underline(false);
    content.set_label(&words::folded(&head.title, open));
    content.set_can_shrink(true);
    let button = gtk::Button::new();
    button.set_child(Some(&content));
    let spoken = words::list_label(&head.title, open);
    button.update_property(&[gtk::accessible::Property::Label(&spoken)]);
    let (lister, list_id) = (lister.clone(), head.list_id.clone());
    on_press(&button, move |by_key| {
        let list_id = list_id.clone();
        act(&lister, ListAct::Open { list_id, by_key });
    });
    button.upcast()
}

/// `text` on one line, cut short with an ellipsis at its end when it has less room than it needs,
/// and spoken whole.
fn one_line(text: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_use_markup(false);
    label.set_text(text);
    label.set_single_line_mode(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_xalign(0.0);
    label.update_property(&[gtk::accessible::Property::Label(text)]);
    label
}

/// The engine's sentence for a refused edit, as written, in a row under the rows of its list,
/// which is not activatable.
fn sentence_row(sentence: &str) -> gtk::ListBoxRow {
    let label = gtk::Label::new(None);
    label.set_use_markup(false);
    label.set_text(sentence);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    label.set_margin_top(8);
    label.set_margin_bottom(8);
    label.set_margin_start(12);
    label.set_margin_end(12);
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_child(Some(&label));
    row
}

/// Removes every widget of the card, the add field among them, which a draw puts back.
fn clear(column: &gtk::Box) {
    while let Some(child) = column.first_child() {
        if let Some(list) = child.downcast_ref::<gtk::ListBox>() {
            list.remove_all();
        }
        column.remove(&child);
    }
}

/// Where the focus is on the card before a draw.
#[derive(Clone, Copy)]
enum Focused {
    /// The row at this place in the card's list.
    Row(i32),
    /// Add an item, which stays focused for the next item.
    Add,
}

/// Where the card holds the focus, if it does.
fn focused_row(column: &gtk::Box, add: &adw::EntryRow) -> Option<Focused> {
    let list = column.first_child()?.downcast::<gtk::ListBox>().ok()?;
    let focus = list.root()?.focus()?;
    if focus.is_ancestor(add) || focus == *add.upcast_ref::<gtk::Widget>() {
        return Some(Focused::Add);
    }
    let row = focus.ancestor(gtk::ListBoxRow::static_type())?;
    let row = row.downcast::<gtk::ListBoxRow>().ok()?;
    row.parent().filter(|parent| parent == &list)?;
    Some(Focused::Row(row.index()))
}

/// After a draw, focus that was on the card goes back: to Add an item when it was there, else to
/// the row now in its place, else the row before, else Add an item. The window's focus is
/// cleared first: GTK moves focus off a removed widget one frame later, which would undo the
/// move.
fn refocus(column: &gtk::Box, focused: Option<Focused>, add: &adw::EntryRow) {
    let Some(focused) = focused else {
        return;
    };
    let Some(list) = column.first_child().and_downcast::<gtk::ListBox>() else {
        return;
    };
    if let Some(root) = list.root() {
        root.set_focus(None::<&gtk::Widget>);
    }
    let row = match focused {
        Focused::Add => None,
        Focused::Row(at) => list.row_at_index(at).or_else(|| list.row_at_index(at - 1)),
    };
    let in_list = add.parent().is_some_and(|parent| parent == list);
    match row {
        Some(row) => row.grab_focus(),
        None if in_list => add.grab_focus(),
        None => list.grab_focus(),
    };
}

#[cfg(test)]
mod tests {
    /// The ways app.rs, main.rs and voice_flow.rs register a window or app action, each followed
    /// by the action's name. voice_flow.rs's `app_action(app,"` is read through `on(app,"`.
    const REGISTERS: [&str; 5] = [
        "on(app,\"",
        "on_plain(app,\"",
        "on_value(app,\"",
        "SimpleAction::new(\"",
        "SimpleAction::new_stateful(\"",
    ];

    /// A tick and an add are their widgets' own closures, and every other ask of a card or the
    /// pane goes to the session through `Lister`: none is a `win.` or `app.` action, which any
    /// process of the session reaches on the bus (M85 lists §4.2). Of every action the app
    /// registers, by any of its ways, the one with "list" in its name is `win.lists`, which only
    /// shows and hides the pane.
    #[test]
    fn no_list_edit_is_a_window_action() {
        let surfaces = [
            ("lists.rs", shipped(include_str!("lists.rs"))),
            ("list_rows.rs", shipped(include_str!("list_rows.rs"))),
            ("lists_pane.rs", shipped(include_str!("lists_pane.rs"))),
        ];
        let read: String = surfaces.iter().map(|(_, source)| *source).collect();
        assert!(
            read.contains("connect_toggled") && read.contains("connect_entry_activated"),
            "the scan reads the edits"
        );
        for (file, source) in surfaces {
            for named in [
                "action_name(",
                "activate_action(",
                "\"win.",
                "\"app.",
                "SimpleAction",
            ] {
                assert!(!source.contains(named), "{file} names {named}");
            }
        }
        let registers = [
            squeezed(include_str!("../app.rs")),
            squeezed(include_str!("../main.rs")),
            squeezed(include_str!("../voice_flow.rs")),
        ];
        let names: Vec<&str> = registers.iter().flat_map(|s| registered(s)).collect();
        let reads = [
            "page",
            "restart",
            "run-in-background",
            "quit",
            "voice-mute",
            "voice-call",
        ];
        for read in reads {
            assert!(names.contains(&read), "the scan reads {read}");
        }
        let lists: Vec<&&str> = names.iter().filter(|name| name.contains("list")).collect();
        assert_eq!(lists, [&"lists"], "win.lists alone");
    }

    /// The name of every action `source` registers in one of the `REGISTERS` ways.
    fn registered(source: &str) -> Vec<&str> {
        let mut names = Vec::new();
        for opens in REGISTERS {
            for (at, _) in source.match_indices(opens) {
                let rest = &source[at + opens.len()..];
                names.extend(rest.split('"').next());
            }
        }
        names
    }

    /// Ctrl+Shift+L is `win.lists`'s, and the Keyboard Shortcuts dialog lists it under Chat
    /// (M85 lists §3.2): the dialog and the two accelerator lists are kept in step.
    #[test]
    fn ctrl_shift_l_shows_or_hides_the_pane_and_the_shortcuts_dialog_lists_it() {
        assert_eq!(crate::app::LISTS_SHORTCUT, "<Ctrl><Shift>l");
        let app = squeezed(include_str!("../app.rs"));
        assert!(app.contains("set_accels_for_action(\"win.lists\",&[LISTS_SHORTCUT])"));
        let main = squeezed(include_str!("../main.rs"));
        assert!(main.contains("chat.add(adw::ShortcutsItem::new(SHOW_OR_HIDE,LISTS_SHORTCUT))"));
    }

    /// A source cut at its test module. The marker is built from two parts, so this file holds it
    /// once, where its tests begin, as the log scan asks.
    fn shipped(source: &str) -> &str {
        let marker = concat!("#[cfg", "(test)]");
        source.split(marker).next().expect("split yields one part")
    }

    /// A source without whitespace, so a call rustfmt broke over lines reads whole. The scan reads
    /// tests too: a test registers no action.
    fn squeezed(source: &str) -> String {
        source.split_whitespace().collect()
    }
}
