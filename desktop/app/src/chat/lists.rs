//! A reply's lists card (M85 lists §2), under its place cards: the owner's lists its turn read or
//! changed, drawn as they stand now (`fermix_client::checklists::card`). A live card of one list
//! shows its open items with "N more" and Add an item, a card of several a group per list, and a
//! folded card one line per list. Only stock widgets, markup off before any text, and no list
//! edit is an exported action: a tick is its check button's own closure, and the card hands each
//! edit to the session through `Lister`.

use adw::prelude::*;
use fermix_client::chat::ListsCard;
use fermix_client::chat_wire::protocol::{ListOp, ListRow};
use fermix_client::checklists::card::{drawing, Drawing, Head, ListGroup, Settling, Stands};
use fermix_client::checklists::cards::OneList;
use fermix_client::checklists::draft::is_local;
use fermix_client::checklists::library::Library;
use fermix_client::checklists::words::{self, ADD_ITEM, LIST_REMOVED, SHOW_LESS};
use gtk::glib;
use std::cell::{Cell, OnceCell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use super::places::{CARDS_MIN_WIDTH, CARDS_WIDTH};

/// How long a ticked row keeps its place before it folds away, with animations on.
const SETTLE: Duration = Duration::from_millis(700);
/// The mark of a list, on a card's head and a folded line.
const LIST_MARK: &str = "checkbox-checked-symbolic";

/// What a reply's lists card asks of the session.
pub enum ListAct {
    /// A live card came on screen: each list it draws is pulled unless the app holds it. The
    /// page asks this while it holds its entries, so the answer draws nothing: a pull changes no
    /// card until its list arrives, and that arrival draws. Asked once each time the card comes
    /// on screen, not at every look.
    Drawn(Vec<String>),
    /// An edit of one list, from a tick or Add an item.
    Edit { list_id: String, op: ListOp },
    /// The card's own state changed (grown, folded back, or a ticked row settled): the cards
    /// are drawn again.
    Redraw,
}

/// Where the cards send what they ask: the session, handed over once it exists.
pub type Lister = Rc<OnceCell<Box<dyn Fn(ListAct)>>>;

fn act(lister: &Lister, asked: ListAct) {
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
    /// changed. `on_screen`: the card was on screen at the page's last look, so a list's removal
    /// is spoken.
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
            announce_news(&self.column, self.drawn.as_ref(), drawing, on_screen);
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
            Drawing::Folded(heads) => self.column.append(&folded_line(heads)),
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
        list.append(&head_row(head, false));
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
            list.append(&head_row(&group.head, true));
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

    /// "N more", or "Show less" on a grown card; none when every open item shows on a card that
    /// never grew.
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
        button.connect_activated(move |_| {
            grown.set(!grown.get());
            act(&lister, ListAct::Redraw);
        });
        Some(button)
    }

    /// An item: its words, and a check that ticks it, which takes no focus; Space on the row
    /// ticks it too. Done items are dimmed, and a row added here that the engine has not named
    /// yet takes no tick.
    fn item_row(&self, list_id: &str, row: &ListRow) -> adw::ActionRow {
        let item = adw::ActionRow::new();
        item.set_use_markup(false);
        item.set_title(&row.text);
        item.set_title_lines(0);
        let check = gtk::CheckButton::new();
        check.set_active(row.is_done());
        check.set_focusable(false);
        check.set_valign(gtk::Align::Center);
        item.add_prefix(&check);
        if row.is_done() {
            item.add_css_class("dimmed");
        }
        item.set_sensitive(!is_local(&row.row_id));
        let spoken = item.upcast_ref::<gtk::Widget>();
        spoken.update_property(&[gtk::accessible::Property::Label(&row.text)]);
        let checked = match row.is_done() {
            true => gtk::AccessibleTristate::True,
            false => gtk::AccessibleTristate::False,
        };
        spoken.update_state(&[gtk::accessible::State::Checked(checked)]);
        self.tick_on_toggle(&check, list_id, &row.row_id);
        tick_on_space(&item, &check);
        item
    }

    /// The check's own closure sends the tick. A row ticked done keeps its place while it
    /// settles, with animations on, then the card is drawn again without it.
    fn tick_on_toggle(&self, check: &gtk::CheckButton, list_id: &str, row_id: &str) {
        let (lister, settling) = (self.lister.clone(), self.settling.clone());
        let (list_id, row_id) = (list_id.to_owned(), row_id.to_owned());
        check.connect_toggled(move |check| {
            let done = check.is_active();
            if done && check.settings().is_gtk_enable_animations() {
                settle(&settling, &lister, &row_id);
            }
            let row_id = row_id.clone();
            let op = match done {
                true => ListOp::Check { row_id },
                false => ListOp::Uncheck { row_id },
            };
            let list_id = list_id.clone();
            act(&lister, ListAct::Edit { list_id, op });
        });
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

/// Keeps `row_id` in its place for `SETTLE`, then, unless a later settle of it is running, asks
/// for the cards to be drawn again.
fn settle(settling: &Rc<RefCell<Settling>>, lister: &Lister, row_id: &str) {
    settling.borrow_mut().begin(row_id);
    let (settling, lister, row_id) = (settling.clone(), lister.clone(), row_id.to_owned());
    glib::timeout_add_local_once(SETTLE, move || {
        let ended = settling.borrow_mut().end(&row_id);
        if ended {
            act(&lister, ListAct::Redraw);
        }
    });
}

/// Space on a focused item ticks it, before the row's own Space activates it.
fn tick_on_space(item: &adw::ActionRow, check: &gtk::CheckButton) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let check = check.downgrade();
    keys.connect_key_pressed(move |_, key, _, state| {
        let plain = (state & gtk::accelerator_get_default_mod_mask()).is_empty();
        let Some(check) = check
            .upgrade()
            .filter(|_| plain && key == gtk::gdk::Key::space)
        else {
            return glib::Propagation::Proceed;
        };
        check.set_active(!check.is_active());
        glib::Propagation::Stop
    });
    item.add_controller(keys);
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
/// (§2.3).
fn head_row(head: &Head, group: bool) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_use_markup(false);
    row.set_title(&head.title);
    let mark = gtk::Image::from_icon_name(LIST_MARK);
    mark.add_css_class("dimmed");
    mark.set_accessible_role(gtk::AccessibleRole::Presentation);
    row.add_prefix(&mark);
    match head.stands {
        Stands::Counted { open, done } => {
            let (counts, spoken) = match group {
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
    row
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

/// A folded card (§2.4): one line per list it drew, with the list's mark, counted from the index;
/// a removed list says so. Labels for now: the pane they would open is not drawn yet. Each part is
/// one line, cut short at its end when it is wider than the card, so a long title never sets the
/// chat's width; a screen reader hears it whole.
fn folded_line(heads: &[Head]) -> adw::WrapBox {
    let line = adw::WrapBox::new();
    line.set_child_spacing(12);
    line.set_line_spacing(6);
    for head in heads {
        line.append(&folded_part(head));
    }
    line
}

fn folded_part(head: &Head) -> gtk::Widget {
    let Stands::Counted { open, .. } = head.stands else {
        let removed = matches!(head.stands, Stands::Removed);
        let label = one_line(if removed { LIST_REMOVED } else { &head.title });
        label.add_css_class("dimmed");
        return label.upcast();
    };
    let part = gtk::Box::builder().spacing(6).build();
    let mark = gtk::Image::from_icon_name(LIST_MARK);
    mark.set_accessible_role(gtk::AccessibleRole::Presentation);
    part.append(&mark);
    part.append(&one_line(&words::folded(&head.title, open)));
    let spoken = words::list_label(&head.title, open);
    part.update_property(&[gtk::accessible::Property::Label(&spoken)]);
    part.upcast()
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

/// A refusal's sentence is spoken when it appears, and so is "This list was removed." on a card
/// of one list that was on screen: focus is elsewhere. A card far up the chat says nothing when
/// its list goes.
fn announce_news(column: &gtk::Box, was: Option<&Drawing>, now: &Drawing, on_screen: bool) {
    let before = was.map(refusals).unwrap_or_default();
    let mut news: Vec<&str> = refusals(now)
        .into_iter()
        .filter(|said| !before.contains(said))
        .collect();
    let removed = removed_of(now).filter(|_| on_screen);
    news.extend(removed.filter(|said| was.and_then(removed_of) != Some(*said)));
    for said in news {
        column.announce(said, gtk::AccessibleAnnouncementPriority::Medium);
    }
}

/// The sentences of the refused edits the card shows: its one list's, or each group's.
fn refusals(drawing: &Drawing) -> Vec<&str> {
    match drawing {
        Drawing::One { refusal, .. } => refusal.as_deref().into_iter().collect(),
        Drawing::Several { groups, .. } => {
            groups.iter().filter_map(|g| g.refusal.as_deref()).collect()
        }
        Drawing::Folded(_) => Vec::new(),
    }
}

fn removed_of(drawing: &Drawing) -> Option<&str> {
    match drawing {
        Drawing::One { head, .. } if head.stands == Stands::Removed => Some(LIST_REMOVED),
        Drawing::One { .. } | Drawing::Several { .. } | Drawing::Folded(_) => None,
    }
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
    /// by the action's name.
    const REGISTERS: [&str; 5] = [
        "on(app,\"",
        "on_plain(app,\"",
        "on_value(app,\"",
        "SimpleAction::new(\"",
        "SimpleAction::new_stateful(\"",
    ];

    /// A card's tick and add are its widgets' own closures, never a `win.` or `app.` action,
    /// which any process of the session reaches on the bus (M85 lists §4.2): the card names no
    /// action, and no action the app registers, by any of its ways, has "list" in its name.
    #[test]
    fn no_list_edit_is_a_window_action() {
        let card = shipped(include_str!("lists.rs"));
        assert!(
            card.contains("connect_toggled") && card.contains("connect_entry_activated"),
            "the scan reads the card's edits"
        );
        for named in [
            "action_name(",
            "activate_action(",
            "\"win.",
            "\"app.",
            "SimpleAction",
        ] {
            assert!(!card.contains(named), "the card names {named}");
        }
        let registers = [
            squeezed(include_str!("../app.rs")),
            squeezed(include_str!("../main.rs")),
            squeezed(include_str!("../voice_flow.rs")),
        ];
        let names: Vec<&str> = registers.iter().flat_map(|s| registered(s)).collect();
        for read in ["page", "restart", "run-in-background", "quit", "voice-mute"] {
            assert!(names.contains(&read), "the scan reads {read}");
        }
        let lists: Vec<&&str> = names.iter().filter(|name| name.contains("list")).collect();
        assert!(lists.is_empty(), "list actions registered: {lists:?}");
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
