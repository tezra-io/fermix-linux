//! The Phone dialog (M60 §3.3): one dialog titled with the daemon's section title, whose one step
//! changes in place. It raises nothing over itself: no second dialog, alert, toast or popover.
//! Every step is the core reducer's (`fermix_client::pairing`); this file only draws it.
//!
//! The pairing link lives in the Scan step and in the widgets drawing it, and goes with them. A
//! copy of it on the clipboard is taken back when Scan leaves, if it is still Fermix's. Linux has
//! no way to keep one window out of a screen share, so nothing here claims to.

use crate::descriptor::SectionView;
use adw::prelude::*;
use fermix_client::mobile::{MobileDevice, MobileDevices};
use fermix_client::pairing::{Compare, PairingCode, Progress, Scan, Step, TurnOn};
use fermix_client::phone::{
    self as words, Forgetting, CANCEL, CANT_SCAN, CODE_LABEL, COMPARE_LINE, COPY_LINK, DONE,
    FORGET, FORGET_CONFIRM, LINK_LABEL, PAIR, PAIR_ANOTHER, RESTARTING, SCAN_LINE, SECTION,
    STATUS_CHECKING, SWITCH_KEY,
};
use gtk::glib::{self, variant::ToVariant};
use gtk::{cairo, gdk, pango};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The width and height every channel dialog has.
pub const WIDTH: i32 = 560;
pub const HEIGHT: i32 = 560;
const MARGIN: i32 = 24;
const _: () = assert!(PairingCode::MAX_SIDE as i32 + 2 * MARGIN <= WIDTH);
/// What a clipboard manager that honours it reads as "do not keep this".
const SECRET_HINT: (&str, &[u8]) = ("x-kde-passwordManagerHint", b"secret");

/// Everything a step draws from: the step, and what the app knows beside it.
pub struct Screen<'a> {
    pub step: &'a Step,
    /// A decision on its way to the daemon, which holds both buttons.
    pub deciding: bool,
    pub forgetting: &'a Forgetting,
    pub devices: Option<&'a Result<MobileDevices, String>>,
    /// The daemon's footer for the channel's switch, which Turn on leads with.
    pub footer: Option<&'a str>,
    /// Unix seconds, for when a phone was last seen.
    pub now: i64,
}

/// What a drawn step was drawn from; a screen equal to it draws nothing.
#[derive(Clone, PartialEq)]
struct Key {
    step: Step,
    deciding: bool,
    forgetting: Forgetting,
    devices: Option<Result<MobileDevices, String>>,
    footer: Option<String>,
}

impl Key {
    fn of(screen: &Screen<'_>) -> Key {
        Key {
            step: screen.step.clone(),
            deciding: screen.deciding,
            forgetting: screen.forgetting.clone(),
            devices: screen.devices.cloned(),
            footer: screen.footer.map(str::to_owned),
        }
    }
}

/// The widgets a step changes in place rather than drawing again.
enum Parts {
    Plain,
    TurnOn(TurnOnParts),
    Scan(ScanParts),
    Compare(CompareParts),
    Phones(PhonesParts),
}

struct TurnOnParts {
    lead: gtk::Box,
    lead_label: gtk::Label,
    working: gtk::Box,
    working_label: gtk::Label,
    refusal: gtk::Label,
    act: gtk::Button,
}

struct ScanParts {
    countdown: gtk::Label,
    /// Whether the last ten seconds have been announced.
    final_announced: Cell<bool>,
    /// This step's copy of the link, while it may still be on the clipboard.
    copied: Rc<RefCell<Option<gdk::ContentProvider>>>,
    clipboard: gdk::Clipboard,
}

struct CompareParts {
    deny: gtk::Button,
    approve: gtk::Button,
}

struct PhonesParts {
    group: adw::PreferencesGroup,
    rows: RefCell<Vec<gtk::Widget>>,
    /// The channel's connection rows, drawn by the one descriptor renderer.
    view: Rc<SectionView>,
}

struct Drawn {
    key: Key,
    parts: Parts,
}

pub struct PhoneView {
    pub dialog: adw::Dialog,
    toolbar: adw::ToolbarView,
    foot: RefCell<Option<gtk::Box>>,
    drawn: RefCell<Option<Drawn>>,
    /// The settings views every render draws; the Phones step's own is added and taken back.
    dialog_views: Rc<RefCell<Vec<Rc<SectionView>>>>,
}

impl PhoneView {
    pub fn new(title: &str, dialog_views: Rc<RefCell<Vec<Rc<SectionView>>>>) -> PhoneView {
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        let dialog = adw::Dialog::builder()
            .title(title)
            .content_width(WIDTH)
            .content_height(HEIGHT)
            .child(&toolbar)
            .build();
        PhoneView {
            dialog,
            toolbar,
            foot: RefCell::default(),
            drawn: RefCell::default(),
            dialog_views,
        }
    }

    /// Draws the screen: in place where only what a step updates changed, and the whole step
    /// where it is another one, with the focus on its heading.
    pub fn show(&self, screen: &Screen<'_>) {
        let key = Key::of(screen);
        let mut drawn = self.drawn.borrow_mut();
        if drawn.as_ref().is_some_and(|d| d.key == key) {
            return;
        }
        if let Some(current) = drawn.as_mut() {
            if update(&current.parts, &current.key, screen) {
                current.key = key;
                return;
            }
        }
        let first = drawn.is_none();
        let changed_kind = drawn
            .as_ref()
            .is_none_or(|d| d.key.step.kind() != screen.step.kind());
        if let Some(old) = drawn.take() {
            self.retire(old.parts);
        }
        let parts = self.build(screen, first || changed_kind);
        *drawn = Some(Drawn { key, parts });
    }

    /// The dialog is going: the Scan step's copy leaves the clipboard and the Phones step's
    /// rows stop being drawn.
    pub fn close_down(&self) {
        if let Some(old) = self.drawn.borrow_mut().take() {
            self.retire(old.parts);
        }
    }

    fn retire(&self, parts: Parts) {
        match parts {
            Parts::Scan(scan) => withdraw(&scan.clipboard, &scan.copied),
            Parts::Phones(phones) => self
                .dialog_views
                .borrow_mut()
                .retain(|v| !Rc::ptr_eq(v, &phones.view)),
            Parts::Plain | Parts::TurnOn(_) | Parts::Compare(_) => {}
        }
    }

    fn build(&self, screen: &Screen<'_>, focus: bool) -> Parts {
        // The focused widget is about to go; with nothing focused, GTK does not move the focus
        // on the next frame and undo the heading's.
        self.dialog.set_focus(None::<&gtk::Widget>);
        let built = match screen.step {
            Step::Waiting { .. } => Built::bare(waiting()),
            Step::TurnOn(turn_on) => turn_on_step(turn_on, screen.footer),
            Step::Scan(scan) => scan_step(scan, &self.dialog.clipboard()),
            Step::Compare(compare) => compare_step(compare, screen.deciding),
            Step::Paired { name } => {
                let done = pill(DONE, "win.phone-phones", true);
                sentence_step(&words::paired_line(name), Vec::new(), done)
            }
            Step::Ended(ending) => {
                let done = pill(DONE, "win.phone-dismiss", false);
                let act = pill(ending.action_title(), "win.phone-ending", true);
                sentence_step(&ending.sentence, vec![done], act)
            }
            Step::Phones => self.phones_step(screen),
        };
        self.toolbar.set_content(Some(&built.content));
        if let Some(old) = self.foot.borrow_mut().take() {
            self.toolbar.remove(&old);
        }
        if let Some(foot) = &built.foot {
            self.toolbar.add_bottom_bar(foot);
        }
        *self.foot.borrow_mut() = built.foot;
        self.dialog.set_default_widget(built.default.as_ref());
        if let (true, Some(lead)) = (focus, &built.lead) {
            focus_when_shown(lead);
        }
        built.parts
    }

    /// Phones: each paired phone, Pair another phone, then the channel's connection rows without
    /// its switch, which stays on the Channels row.
    fn phones_step(&self, screen: &Screen<'_>) -> Built {
        let page = adw::PreferencesPage::new();
        let group = adw::PreferencesGroup::new();
        page.add(&group);
        let keep = Box::new(|key: &str| key != SWITCH_KEY);
        let view = SectionView::keeping(SECTION, None, keep);
        page.add(&view.group);
        self.dialog_views.borrow_mut().push(view.clone());
        let phones = PhonesParts {
            group,
            rows: RefCell::default(),
            view,
        };
        phones.fill(screen);
        let done = pill(DONE, "win.phone-dismiss", true);
        Built {
            content: page.upcast(),
            foot: Some(foot(std::slice::from_ref(&done))),
            parts: Parts::Phones(phones),
            lead: None,
            default: Some(done),
        }
    }
}

/// One step as drawn.
struct Built {
    content: gtk::Widget,
    /// The step's buttons, at the foot of the dialog.
    foot: Option<gtk::Box>,
    parts: Parts,
    /// The heading the focus moves to.
    lead: Option<gtk::Box>,
    /// The step's one primary action, which Return takes.
    default: Option<gtk::Button>,
}

impl Built {
    fn bare(content: gtk::Widget) -> Built {
        Built {
            content,
            foot: None,
            parts: Parts::Plain,
            lead: None,
            default: None,
        }
    }
}

/// Changes a step in place where the step is the same one and only what it updates moved.
/// False where it must be drawn again.
fn update(parts: &Parts, was: &Key, screen: &Screen<'_>) -> bool {
    match (parts, &was.step, screen.step) {
        (Parts::TurnOn(p), Step::TurnOn(_), Step::TurnOn(t)) => {
            p.show(t, screen.footer);
            true
        }
        (Parts::Scan(p), Step::Scan(old), Step::Scan(new)) if old.session == new.session => {
            p.tick(new.ttl_ms);
            true
        }
        (Parts::Compare(p), Step::Compare(old), Step::Compare(new))
            if old.session == new.session =>
        {
            p.deny.set_sensitive(!screen.deciding);
            p.approve.set_sensitive(!screen.deciding);
            true
        }
        (Parts::Phones(p), Step::Phones, Step::Phones) => {
            p.fill(screen);
            true
        }
        (Parts::Plain, Step::Waiting { .. }, Step::Waiting { .. }) => true,
        _ => false,
    }
}

fn waiting() -> gtk::Widget {
    let spinner = adw::Spinner::builder()
        .width_request(32)
        .height_request(32)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    spinner.upcast()
}

/// A step's column: centred, its parts spaced alike.
fn column() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .valign(gtk::Align::Center)
        .margin_top(MARGIN)
        .margin_bottom(MARGIN)
        .margin_start(MARGIN)
        .margin_end(MARGIN)
        .build()
}

/// A step's lead line, where the focus goes as the step changes (§7). A label takes no focus
/// unless it is selectable, so a focusable box carries the heading role and the words.
fn heading(text: &str) -> (gtk::Box, gtk::Label) {
    let label = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .hexpand(true)
        .justify(gtk::Justification::Center)
        .css_classes(["title-4"])
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build();
    let lead = gtk::Box::builder()
        .focusable(true)
        .halign(gtk::Align::Fill)
        .accessible_role(gtk::AccessibleRole::Heading)
        .build();
    lead.update_property(&[gtk::accessible::Property::Label(text)]);
    lead.append(&label);
    (lead, label)
}

fn note(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .wrap(true)
        .justify(gtk::Justification::Center)
        .css_classes(["dim-label"])
        .build()
}

/// The heading takes the focus once it is on screen, on the next idle turn.
fn focus_when_shown(lead: &gtk::Box) {
    lead.connect_map(|lead| {
        let lead = lead.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(lead) = lead.upgrade() {
                lead.grab_focus();
            }
        });
    });
}

fn pill(label: &str, action: &str, primary: bool) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(label)
        .action_name(action)
        .css_classes(["pill"])
        .build();
    if primary {
        button.add_css_class("suggested-action");
    }
    button
}

/// The step's buttons, side by side at the foot of the dialog.
fn foot(buttons: &[gtk::Button]) -> gtk::Box {
    let foot = gtk::Box::builder()
        .spacing(12)
        .halign(gtk::Align::Center)
        .margin_top(12)
        .margin_bottom(MARGIN)
        .margin_start(MARGIN)
        .margin_end(MARGIN)
        .build();
    for button in buttons {
        foot.append(button);
    }
    foot
}

/// Turn on: the daemon's own footer for the switch, what a restart interrupts, and the one
/// button, which says it restarts.
fn turn_on_step(turn_on: &TurnOn, footer: Option<&str>) -> Built {
    let body = column();
    let (lead, lead_label) = heading(footer.unwrap_or_default());
    body.append(&lead);
    body.append(&note(crate::service::INTERRUPTS));
    let working_label = gtk::Label::new(Some(RESTARTING));
    working_label.add_css_class("dim-label");
    let working = gtk::Box::builder()
        .spacing(12)
        .halign(gtk::Align::Center)
        .build();
    working.append(&adw::Spinner::new());
    working.append(&working_label);
    body.append(&working);
    let refusal = note("");
    refusal.set_css_classes(&["error"]);
    body.append(&refusal);
    let act = pill(turn_on.action_title(), "win.phone-turn-on", true);
    let foot = foot(&[pill(CANCEL, "win.phone-dismiss", false), act.clone()]);
    let parts = TurnOnParts {
        lead: lead.clone(),
        lead_label,
        working,
        working_label,
        refusal,
        act,
    };
    parts.show(turn_on, footer);
    Built {
        content: body.upcast(),
        foot: Some(foot),
        default: Some(parts.act.clone()),
        parts: Parts::TurnOn(parts),
        lead: Some(lead),
    }
}

impl TurnOnParts {
    fn show(&self, turn_on: &TurnOn, footer: Option<&str>) {
        let lead = footer.unwrap_or_default();
        self.lead_label.set_label(lead);
        self.lead
            .update_property(&[gtk::accessible::Property::Label(lead)]);
        self.lead.set_visible(!lead.is_empty());
        self.working.set_visible(turn_on.progress != Progress::Idle);
        self.working_label
            .set_visible(turn_on.progress == Progress::Restarting);
        let refusal = turn_on.refusal.as_deref().unwrap_or_default();
        self.refusal.set_label(refusal);
        self.refusal.set_visible(!refusal.is_empty());
        self.act.set_label(turn_on.action_title());
        self.act.set_sensitive(turn_on.progress == Progress::Idle);
    }
}

/// Scan: the code on its white card, the one line, the daemon's own countdown, and the link for
/// a phone that cannot scan.
fn scan_step(scan: &Scan, clipboard: &gdk::Clipboard) -> Built {
    let body = column();
    let (lead, _) = heading(SCAN_LINE);
    body.append(&lead);
    body.append(&code_card(&scan.code));
    let countdown = gtk::Label::builder()
        .label(words::countdown(scan.ttl_ms))
        .css_classes(["dim-label", "numeric"])
        .build();
    body.append(&countdown);
    let copied = Rc::default();
    body.append(&link_reveal(scan.link.text(), clipboard, &copied));
    let foot = foot(&[pill(CANCEL, "win.phone-dismiss", false)]);
    let parts = ScanParts {
        countdown,
        final_announced: Cell::new(false),
        copied,
        clipboard: clipboard.clone(),
    };
    // The countdown is spoken as Scan appears and once more at the last ten seconds (§7).
    parts.countdown.connect_map(|label| {
        label.announce(&label.label(), gtk::AccessibleAnnouncementPriority::Medium);
    });
    parts.tick(scan.ttl_ms);
    Built {
        content: body.upcast(),
        foot: Some(foot),
        parts: Parts::Scan(parts),
        lead: Some(lead),
        default: None,
    }
}

impl ScanParts {
    fn tick(&self, ttl_ms: i64) {
        let text = words::countdown(ttl_ms);
        self.countdown.set_label(&text);
        if words::final_countdown(ttl_ms) && !self.final_announced.replace(true) {
            self.countdown
                .announce(&text, gtk::AccessibleAnnouncementPriority::Medium);
        }
    }
}

/// The code, black on white in both appearances, since a camera needs the contrast. Each module
/// is a whole number of pixels, drawn with no smoothing. It is labelled and carries no value:
/// what it draws is the secret.
fn code_card(code: &PairingCode) -> gtk::DrawingArea {
    let side = i32::try_from(code.side()).expect("a code fits the dialog");
    let card = gtk::DrawingArea::builder()
        .content_width(side)
        .content_height(side)
        .halign(gtk::Align::Center)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["card"])
        .accessible_role(gtk::AccessibleRole::Img)
        .build();
    card.update_property(&[gtk::accessible::Property::Label(CODE_LABEL)]);
    let code = code.clone();
    card.set_draw_func(move |_, cr, _, _| {
        if let Err(e) = draw_code(cr, &code) {
            glib::g_warning!("fermix", "the pairing code was not drawn: {e}");
        }
    });
    card
}

fn draw_code(cr: &cairo::Context, code: &PairingCode) -> Result<(), cairo::Error> {
    let px = code.module_px() as f64;
    let zone = PairingCode::QUIET_ZONE as f64;
    cr.set_antialias(cairo::Antialias::None);
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.paint()?;
    cr.set_source_rgb(0.0, 0.0, 0.0);
    for row in 0..code.width() {
        for column in (0..code.width()).filter(|&c| code.dark(row, c)) {
            cr.rectangle(
                (zone + column as f64) * px,
                (zone + row as f64) * px,
                px,
                px,
            );
        }
    }
    cr.fill()
}

/// "Can't scan the code?", which shows the link as text with Copy link. The text cannot be
/// selected: Copy is the one way it leaves the dialog, since only Copy marks it secret and takes
/// it back when Scan leaves.
fn link_reveal(
    link: &str,
    clipboard: &gdk::Clipboard,
    copied: &Rc<RefCell<Option<gdk::ContentProvider>>>,
) -> gtk::Box {
    let text = gtk::Label::builder()
        .label(link)
        .selectable(false)
        .wrap(true)
        .wrap_mode(pango::WrapMode::Char)
        .lines(3)
        .ellipsize(pango::EllipsizeMode::Middle)
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["monospace"])
        .build();
    text.update_property(&[gtk::accessible::Property::Label(LINK_LABEL)]);
    let copy = gtk::Button::builder()
        .label(COPY_LINK)
        .valign(gtk::Align::Center)
        .build();
    let (board, mine, value) = (clipboard.clone(), copied.clone(), link.to_owned());
    copy.connect_clicked(move |_| copy_secret(&board, &mine, &value));
    let shown = gtk::Box::builder()
        .spacing(12)
        .css_classes(["card"])
        .visible(false)
        .build();
    text.set_margin_start(12);
    text.set_margin_top(8);
    text.set_margin_bottom(8);
    copy.set_margin_end(8);
    shown.append(&text);
    shown.append(&copy);
    let reveal = gtk::Button::builder()
        .label(CANT_SCAN)
        .halign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let link_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    link_box.append(&reveal);
    link_box.append(&shown);
    let target = shown.downgrade();
    reveal.connect_clicked(move |reveal| {
        reveal.set_visible(false);
        if let Some(shown) = target.upgrade() {
            shown.set_visible(true);
        }
    });
    link_box
}

/// Offers the link as text with the secret hint beside it, and remembers the offer so Scan can
/// take it back.
fn copy_secret(
    clipboard: &gdk::Clipboard,
    copied: &Rc<RefCell<Option<gdk::ContentProvider>>>,
    link: &str,
) {
    let (mime, hint) = SECRET_HINT;
    let provider = gdk::ContentProvider::new_union(&[
        gdk::ContentProvider::for_value(&link.to_value()),
        gdk::ContentProvider::for_bytes(mime, &glib::Bytes::from_static(hint)),
    ]);
    match clipboard.set_content(Some(&provider)) {
        Ok(()) => *copied.borrow_mut() = Some(provider),
        Err(e) => glib::g_warning!("fermix", "the pairing link was not copied: {e}"),
    }
}

/// Takes this step's copy off the clipboard, if the clipboard still holds it.
fn withdraw(clipboard: &gdk::Clipboard, copied: &Rc<RefCell<Option<gdk::ContentProvider>>>) {
    let Some(mine) = copied.borrow_mut().take() else {
        return;
    };
    if !clipboard.is_local() || clipboard.content().as_ref() != Some(&mine) {
        return;
    }
    if let Err(e) = clipboard.set_content(None::<&gdk::ContentProvider>) {
        glib::g_warning!(
            "fermix",
            "the pairing link was not taken off the clipboard: {e}"
        );
    }
}

/// Compare: the phone, its six digits large and grouped as the phone draws them, and the
/// decision. Deny and Approve are drawn alike and neither is the default, so Return approves
/// nothing.
fn compare_step(compare: &Compare, deciding: bool) -> Built {
    let body = column();
    let (lead, _) = heading(&words::heading(&compare.device_name));
    body.append(&pair_of(&lead, &note(&compare.model)));
    let digits = gtk::Label::builder()
        .label(words::grouped(&compare.digits))
        .css_classes(["title-1", "numeric"])
        .build();
    let spoken = words::spoken(&compare.digits, &compare.device_name);
    digits.update_property(&[gtk::accessible::Property::Label(&spoken)]);
    body.append(&digits);
    let line = gtk::Label::builder()
        .label(COMPARE_LINE)
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    body.append(&pair_of(&line, &note(&compare.hardware)));
    let deny = pill(words::DENY, "win.phone-decide", false);
    deny.set_action_target_value(Some(&false.to_variant()));
    let approve = pill(words::APPROVE, "win.phone-decide", false);
    approve.set_action_target_value(Some(&true.to_variant()));
    deny.set_sensitive(!deciding);
    approve.set_sensitive(!deciding);
    let foot = foot(&[deny.clone(), approve.clone()]);
    Built {
        content: body.upcast(),
        foot: Some(foot),
        parts: Parts::Compare(CompareParts { deny, approve }),
        lead: Some(lead),
        default: None,
    }
}

/// A line and the note under it, held closer than the step's parts.
fn pair_of(line: &impl IsA<gtk::Widget>, under: &gtk::Label) -> gtk::Box {
    let pair = gtk::Box::new(gtk::Orientation::Vertical, 6);
    pair.append(line);
    pair.append(under);
    pair
}

/// Paired and Ended: one sentence, then `others` and the one way on, `act`, which Return takes.
/// Done on Paired goes on to the phones, the one just paired among them.
fn sentence_step(sentence: &str, mut others: Vec<gtk::Button>, act: gtk::Button) -> Built {
    let body = column();
    let (lead, _) = heading(sentence);
    body.append(&lead);
    others.push(act.clone());
    Built {
        content: body.upcast(),
        foot: Some(foot(&others)),
        parts: Parts::Plain,
        lead: Some(lead),
        default: Some(act),
    }
}

impl PhonesParts {
    /// The phones as the daemon lists them, then Pair another phone. A press in a row draws the
    /// rows again; focus that was in them goes to the asking row's Cancel, else to Pair.
    fn fill(&self, screen: &Screen<'_>) {
        let had_focus = self.group.focus_child().is_some();
        if let (true, Some(root)) = (had_focus, self.group.root()) {
            root.set_focus(None::<&gtk::Widget>);
        }
        for old in self.rows.borrow_mut().drain(..) {
            self.group.remove(&old);
        }
        let (mut rows, cancel) = phone_rows(screen);
        let paired_any = matches!(screen.devices, Some(Ok(list)) if !list.devices.is_empty());
        let pair = adw::ButtonRow::builder()
            .title(if paired_any { PAIR_ANOTHER } else { PAIR })
            .action_name("win.phone-pair-another")
            .sensitive(screen.forgetting.forgetting().is_none())
            .build();
        rows.push(pair.clone().upcast());
        for row in &rows {
            self.group.add(row);
        }
        *self.rows.borrow_mut() = rows;
        if had_focus {
            let target = cancel.unwrap_or_else(|| pair.upcast()).downgrade();
            glib::idle_add_local_once(move || {
                if let Some(target) = target.upgrade() {
                    target.grab_focus();
                }
            });
        }
    }
}

/// The phone rows, and the Cancel of the row asking to forget, where one asks.
fn phone_rows(screen: &Screen<'_>) -> (Vec<gtk::Widget>, Option<gtk::Widget>) {
    let list = match screen.devices {
        None => return (vec![checking_row()], None),
        Some(Err(sentence)) => return (vec![sentence_row(sentence)], None),
        Some(Ok(list)) => list,
    };
    let mut cancel = None;
    let mut rows = Vec::new();
    for device in &list.devices {
        let (row, asking) = phone_row(device, screen.forgetting, screen.now);
        cancel = cancel.or(asking);
        rows.push(row.upcast());
    }
    (rows, cancel)
}

fn checking_row() -> gtk::Widget {
    let row = adw::ActionRow::builder().title(STATUS_CHECKING).build();
    row.add_suffix(&adw::Spinner::new());
    row.upcast()
}

fn sentence_row(sentence: &str) -> gtk::Widget {
    let row = adw::ActionRow::new();
    row.set_title(&glib::markup_escape_text(sentence));
    row.add_css_class("setting-refused");
    row.upcast()
}

/// One paired phone: its name, its model and when it was seen, and Forget, which asks in the row
/// before anything is forgotten. A refusal takes the line under the name, in the daemon's words.
/// While the row asks, its Cancel comes back too.
fn phone_row(
    device: &MobileDevice,
    forgetting: &Forgetting,
    now: i64,
) -> (adw::ActionRow, Option<gtk::Widget>) {
    let refusal = forgetting.refusal(&device.device_id);
    let detail = words::detail(&device.model, device.last_seen.as_deref(), now);
    let row = adw::ActionRow::new();
    row.set_title(&glib::markup_escape_text(&device.name));
    row.set_subtitle(&glib::markup_escape_text(refusal.unwrap_or(&detail)));
    if refusal.is_some() {
        row.add_css_class("setting-refused");
    }
    let idle = forgetting.forgetting().is_none();
    let asking = forgetting.asking() == Some(device.device_id.as_str());
    let buttons = if asking {
        let confirm = row_button(FORGET_CONFIRM, "win.phone-forget", &device.name);
        confirm.add_css_class("destructive-action");
        vec![confirm, row_button(CANCEL, "win.phone-forget-withdraw", "")]
    } else {
        let ask = row_button(FORGET, "win.phone-forget-ask", &device.name);
        ask.set_action_target_value(Some(&device.device_id.to_variant()));
        vec![ask]
    };
    for button in &buttons {
        button.set_sensitive(idle);
        row.add_suffix(button);
    }
    let cancel = asking.then(|| buttons[1].clone().upcast());
    (row, cancel)
}

/// A row's button; where it acts on one phone, its accessible name says which.
fn row_button(label: &str, action: &str, phone: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(label)
        .action_name(action)
        .valign(gtk::Align::Center)
        .build();
    if !phone.is_empty() {
        let name = format!("{label}, {phone}");
        button.update_property(&[gtk::accessible::Property::Label(&name)]);
    }
    button
}

#[cfg(test)]
mod tests {
    /// The phone's surfaces name its platform only in core's two strings, the Scan line and
    /// the setup row; nothing drawn here spells it again.
    #[test]
    fn the_phone_surfaces_never_spell_the_platform() {
        let platform = concat!("Andr", "oid");
        for (file, source) in [
            ("phone.rs", include_str!("phone.rs")),
            ("phone_flow.rs", include_str!("phone_flow.rs")),
            ("channels.rs", include_str!("channels.rs")),
            ("assistant.rs", include_str!("assistant.rs")),
        ] {
            assert!(!source.contains(platform), "{file}");
        }
    }
}
