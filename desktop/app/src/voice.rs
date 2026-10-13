//! The Pet page (spec_voice §4.3), top to bottom, as the macOS Pet tab: the pet's
//! still mark, with one line under it and one button: before a call the line says
//! what stands in the way, or why the last one failed, and the button is the one
//! thing that fixes it, or Begin; the call's transcript; the live-call facts; the
//! microphone a call records from, with its meter during a call and Linux's
//! microphone statement folded under it; and the companion window, where the pet
//! moves. It draws from `VoiceView` only; the controller decides everything.

use adw::prelude::*;
use fermix_client::ledger::{MICROPHONE_DETAIL, MICROPHONE_HEADLINE};
use fermix_client::mascot::Expression;
use fermix_client::realtime::session::{Palette, TranscriptLine};
use fermix_client::voice::{GateAction, StatusLine, VoiceGate};
use gtk::glib::{self, variant::ToVariant};
use std::cell::RefCell;

/// Everything the page shows, decided by the controller.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceView {
    pub gate: VoiceGate,
    /// Begin is offered: nothing but the last attempt's answer is in the way.
    pub can_begin: bool,
    /// The line under the mascot; the mascot draws in its palette.
    pub status: StatusLine,
    pub expression: Expression,
    /// The smoothed output level, 0 to 1, while Fermix speaks.
    pub level: f32,
    /// The microphone meter, 0 to 1, while the microphone streams.
    pub mic_level: Option<f32>,
    pub in_call: bool,
    pub muted: bool,
    /// Stop is offered only while Fermix thinks or speaks.
    pub can_stop: bool,
    pub can_cancel_task: bool,
    /// A call is starting or ending: the call button waits.
    pub busy: bool,
    /// The call's transcript, oldest first; it stays until the next call begins.
    pub transcript: Vec<TranscriptLine>,
    pub task: Option<String>,
    pub usage: Option<String>,
    /// The Microphone row's value: the input a call records from, "None" or "Unknown".
    pub microphone: String,
}

impl VoiceView {
    /// The levels change many times a second while someone speaks; everything
    /// else changes rarely. Only a real change redraws the page.
    pub fn same_apart_from_levels(&self, other: &VoiceView) -> bool {
        let mut this = self.clone();
        this.level = other.level;
        this.mic_level = other.mic_level;
        this == *other
    }
}

pub struct VoicePage {
    pub root: adw::PreferencesPage,
    pub companion: adw::SwitchRow,
    status: gtk::Label,
    status_icon: gtk::Image,
    call: gtk::Button,
    mute: gtk::ToggleButton,
    stop: gtk::Button,
    /// Takes the call button's place while the gate has a fix to offer.
    fix: gtk::Button,
    transcript: adw::PreferencesGroup,
    /// The transcript's rows as drawn, to take out when it changes.
    transcript_rows: RefCell<Vec<adw::ActionRow>>,
    live: adw::PreferencesGroup,
    task: adw::ActionRow,
    cancel_task: gtk::Button,
    usage: adw::ActionRow,
    microphone: adw::ActionRow,
    meter: gtk::LevelBar,
    /// What was drawn last, so new levels alone redraw nothing but the meter:
    /// the page shows the pet still, and only the companion follows the voice.
    shown: RefCell<Option<VoiceView>>,
}

impl VoicePage {
    pub fn new() -> VoicePage {
        let (call_group, status, status_icon) = call_group();
        let (call, mute, stop) = controls();
        let fix = gtk::Button::builder()
            .css_classes(["pill", "suggested-action"])
            .visible(false)
            .build();
        fix.set_action_name(Some("win.voice-fix"));
        call_group.add(&controls_bar(&call, &fix, &mute, &stop));
        let transcript = adw::PreferencesGroup::builder()
            .title("Transcript")
            .visible(false)
            .build();
        let (live, task, usage) = live_group();
        let cancel_task = gtk::Button::builder()
            .label("Cancel Task")
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        cancel_task.set_action_name(Some("app.voice-cancel-task"));
        task.add_suffix(&cancel_task);
        let companion = adw::SwitchRow::builder()
            .title("Companion window")
            .subtitle(COMPANION_NOTE)
            .build();
        let (microphone_group, microphone, meter) = microphone_group();
        let root = adw::PreferencesPage::new();
        for group in [&call_group, &transcript, &live, &microphone_group] {
            root.add(group);
        }
        root.add(&more_group(&companion));
        VoicePage {
            root,
            companion,
            status,
            status_icon,
            call,
            mute,
            stop,
            fix,
            transcript,
            transcript_rows: RefCell::default(),
            live,
            task,
            cancel_task,
            usage,
            microphone,
            meter,
            shown: RefCell::default(),
        }
    }

    pub fn render(&self, view: &VoiceView) {
        self.meter.set_visible(view.mic_level.is_some());
        self.meter
            .set_value(f64::from(view.mic_level.unwrap_or_default()));
        let unchanged = self
            .shown
            .borrow()
            .as_ref()
            .is_some_and(|last| last.same_apart_from_levels(view));
        if unchanged {
            return;
        }
        *self.shown.borrow_mut() = Some(view.clone());
        self.render_status(&view.status);
        self.render_controls(view);
        self.render_transcript(&view.transcript);
        self.render_live(view);
        self.microphone
            .set_subtitle(&glib::markup_escape_text(&view.microphone));
    }

    /// The line under the mascot: a mode's word as a heading beside its icon,
    /// in the mode's colour; a sentence as quiet body text, its words enough.
    fn render_status(&self, line: &StatusLine) {
        self.status.set_text(&line.text);
        self.status_icon.set_visible(line.icon.is_some());
        self.status_icon.set_icon_name(line.icon);
        let (heading, tone) = match line.icon {
            Some(_) => (true, tone(line.palette)),
            None => (false, None),
        };
        set_class(&self.status, "title-4", heading);
        set_class(&self.status, "dimmed", !heading);
        for widget in [
            self.status.upcast_ref::<gtk::Widget>(),
            self.status_icon.upcast_ref(),
        ] {
            for class in TONES {
                set_class(widget, class, tone == Some(class));
            }
        }
    }

    /// One button under the line: what fixes what the line says, or the call.
    fn render_controls(&self, view: &VoiceView) {
        let fix = (!view.in_call).then_some(view.gate.action).flatten();
        self.fix.set_visible(fix.is_some());
        if let Some(action) = fix {
            self.fix.set_label(action_verb(action));
        }
        self.call.set_visible(fix.is_none());
        // Begin is the page's main action only when nothing else has to happen first.
        let (label, classes): (&str, &[&str]) = if view.in_call {
            ("End Voice Call", &["pill", "destructive-action"])
        } else if view.gate.ready {
            ("Begin Voice Call", &["pill", "suggested-action"])
        } else {
            ("Begin Voice Call", &["pill"])
        };
        self.call.set_label(label);
        self.call.set_css_classes(classes);
        self.call
            .set_sensitive(!view.busy && (view.in_call || view.can_begin));
        // A toggle keeps one icon: pressed is muted, and the word says so too.
        // It follows the stateful `app.voice-mute`, which follows the call.
        self.mute.set_visible(view.in_call);
        // Stop stays in place through the call, so End never moves under the pointer.
        self.stop.set_visible(view.in_call);
        self.stop.set_sensitive(view.can_stop);
    }

    /// One property row per line: who spoke above, their words below. The words
    /// are the provider's transcription, never markup.
    fn render_transcript(&self, lines: &[TranscriptLine]) {
        for row in self.transcript_rows.borrow_mut().drain(..) {
            self.transcript.remove(&row);
        }
        let mut rows = self.transcript_rows.borrow_mut();
        for line in lines {
            let row = adw::ActionRow::new();
            row.set_use_markup(false);
            row.set_title(line.who());
            row.set_subtitle(&line.text);
            row.set_subtitle_selectable(true);
            row.add_css_class("property");
            self.transcript.add(&row);
            rows.push(row);
        }
        self.transcript.set_visible(!lines.is_empty());
    }

    fn render_live(&self, view: &VoiceView) {
        let rows = [(&self.task, &view.task), (&self.usage, &view.usage)];
        let mut any = false;
        for (row, value) in rows {
            row.set_visible(value.is_some());
            row.set_subtitle(&glib::markup_escape_text(value.as_deref().unwrap_or("")));
            any |= value.is_some();
        }
        self.cancel_task.set_visible(view.can_cancel_task);
        self.live.set_visible(view.in_call && any);
    }
}

/// The companion window's honest footer (spec_voice §2.4): Fermix cannot keep it
/// on top, so it names what can.
const COMPANION_NOTE: &str = "A small window you can keep beside your work. On GNOME, Alt+Space \
    then Always on Top keeps it above other windows.";

/// The microphone meter's length, in pixels, beside the microphone's name.
const METER_WIDTH: i32 = 96;

/// The pet's still mark, in points, as the macOS Pet tab draws it (M `PetMark.size`).
const PET_MARK_SIZE: i32 = 108;

/// Adwaita's status colours; each mode also has its word and icon.
pub const TONES: [&str; 4] = ["accent", "warning", "success", "error"];

pub fn tone(palette: Palette) -> Option<&'static str> {
    match palette {
        Palette::Accent => Some("accent"),
        Palette::Warning => Some("warning"),
        Palette::Success => Some("success"),
        Palette::Error => Some("error"),
        Palette::Faint | Palette::Secondary => None,
    }
}

fn set_class(widget: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

fn action_verb(action: GateAction) -> &'static str {
    match action {
        GateAction::StartFermix => "Start Fermix",
        GateAction::TurnOn => "Turn On Voice",
        GateAction::AddKey => "Add Key…",
        GateAction::Restart => "Restart Fermix…",
    }
}

/// The microphone a call records from, with a meter of what it sends during a
/// call, and under it M38 §6.5's statement, above every control that opens the
/// microphone: its title always in view, what it says one click away.
fn microphone_group() -> (adw::PreferencesGroup, adw::ActionRow, gtk::LevelBar) {
    let microphone = adw::ActionRow::builder()
        .title("Microphone")
        .subtitle_lines(1)
        .css_classes(["property"])
        .build();
    let meter = gtk::LevelBar::builder()
        .max_value(1.0)
        .width_request(METER_WIDTH)
        .valign(gtk::Align::Center)
        .visible(false)
        .build();
    // One fill colour: a level is not good or bad, only there or not.
    for offset in [
        gtk::LEVEL_BAR_OFFSET_LOW,
        gtk::LEVEL_BAR_OFFSET_HIGH,
        gtk::LEVEL_BAR_OFFSET_FULL,
    ] {
        meter.remove_offset_value(Some(offset));
    }
    meter.update_property(&[gtk::accessible::Property::Label("Microphone level")]);
    microphone.add_suffix(&meter);
    let detail = gtk::Label::builder()
        .label(MICROPHONE_DETAIL)
        .selectable(true)
        .wrap(true)
        .xalign(0.0)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .css_classes(["dimmed"])
        .build();
    let statement = adw::ExpanderRow::builder()
        .title(MICROPHONE_HEADLINE)
        .build();
    statement.add_row(&detail);
    let group = adw::PreferencesGroup::new();
    group.add(&microphone);
    group.add(&statement);
    (group, microphone, meter)
}

/// The pet's still mark with its line under it: a word beside an icon, or a
/// sentence that wraps to a few centred lines.
fn call_group() -> (adw::PreferencesGroup, gtk::Label, gtk::Image) {
    let stage = gtk::Box::builder()
        .halign(gtk::Align::Center)
        .margin_top(12)
        .build();
    stage.append(&pet_mark());
    let status_icon = gtk::Image::new();
    let status = gtk::Label::builder()
        .css_classes(["title-4"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .max_width_chars(48)
        .build();
    let line = gtk::Box::builder()
        .spacing(8)
        .halign(gtk::Align::Center)
        .build();
    line.append(&status_icon);
    line.append(&status);
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .build();
    column.append(&stage);
    column.append(&line);
    let group = adw::PreferencesGroup::new();
    group.add(&column);
    (group, status, status_icon)
}

/// The macOS Pet tab's preview (M `PetMark`): the one-ink pet, 108 points, in
/// the text colour. Decorative: the line under it says the state in words.
fn pet_mark() -> gtk::Image {
    gtk::Image::builder()
        .icon_name("io.tezra.Fermix-symbolic")
        .pixel_size(PET_MARK_SIZE)
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build()
}

fn controls() -> (gtk::Button, gtk::ToggleButton, gtk::Button) {
    let call = gtk::Button::builder().label("Begin Voice Call").build();
    call.set_action_name(Some("app.voice-call"));
    let mute = gtk::ToggleButton::builder()
        .icon_name("microphone-disabled-symbolic")
        .tooltip_text("Mute Microphone")
        .css_classes(["circular"])
        .valign(gtk::Align::Center)
        .build();
    mute.set_action_name(Some("app.voice-mute"));
    let stop = gtk::Button::builder()
        .icon_name("media-playback-stop-symbolic")
        .tooltip_text("Stop the Reply")
        .css_classes(["circular"])
        .valign(gtk::Align::Center)
        .build();
    stop.set_action_name(Some("app.voice-stop"));
    (call, mute, stop)
}

/// The call button and the fix share one place: only one of them shows.
fn controls_bar(
    call: &gtk::Button,
    fix: &gtk::Button,
    mute: &gtk::ToggleButton,
    stop: &gtk::Button,
) -> gtk::Box {
    let bar = gtk::Box::builder()
        .spacing(12)
        .halign(gtk::Align::Center)
        .margin_top(12)
        .margin_bottom(6)
        .build();
    bar.append(mute);
    bar.append(call);
    bar.append(fix);
    bar.append(stop);
    bar
}

fn live_group() -> (adw::PreferencesGroup, adw::ActionRow, adw::ActionRow) {
    // Property rows: a small title, the value below it.
    let row = |title: &str| {
        adw::ActionRow::builder()
            .title(title)
            .css_classes(["property"])
            .subtitle_selectable(true)
            .visible(false)
            .build()
    };
    let (task, usage) = (row("Task"), row("Voice so far"));
    let group = adw::PreferencesGroup::builder()
        .title("This call")
        .visible(false)
        .build();
    for r in [&task, &usage] {
        group.add(r);
    }
    (group, task, usage)
}

fn more_group(companion: &adw::SwitchRow) -> adw::PreferencesGroup {
    let settings = adw::ActionRow::builder()
        .title("Voice settings")
        .subtitle("Model, voice, limits and the OpenAI key")
        .activatable(true)
        .build();
    settings.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    settings.set_action_name(Some("win.page"));
    settings.set_action_target_value(Some(&"voice".to_variant()));
    let group = adw::PreferencesGroup::new();
    group.add(companion);
    group.add(&settings);
    group
}
