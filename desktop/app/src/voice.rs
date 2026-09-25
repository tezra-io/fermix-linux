//! The Voice page (spec_voice §4.3), top to bottom: the microphone statement,
//! the mascot and its status, the call controls, the one row saying what stands
//! between the person and a call, the live-call facts, and the companion window.
//! It draws from `VoiceView` only; the controller decides everything.

use crate::mascot::Mascot;
use adw::prelude::*;
use fermix_client::ledger::MICROPHONE_STATEMENT;
use fermix_client::mascot::Expression;
use fermix_client::realtime::session::Palette;
use fermix_client::voice::{GateAction, VoiceGate};
use gtk::glib::{self, variant::ToVariant};
use std::cell::RefCell;

/// Everything the page shows, decided by the controller.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceView {
    pub gate: VoiceGate,
    /// Begin is offered: nothing but the last attempt's answer is in the way.
    pub can_begin: bool,
    pub word: String,
    pub icon: &'static str,
    pub palette: Palette,
    pub expression: Expression,
    /// The smoothed output level, 0 to 1, while Fermix speaks.
    pub level: f32,
    pub in_call: bool,
    pub muted: bool,
    /// Stop is offered only while Fermix thinks or speaks.
    pub can_stop: bool,
    pub can_cancel_task: bool,
    /// A call is starting or ending: the call button waits.
    pub busy: bool,
    pub caption: Option<String>,
    pub task: Option<String>,
    pub usage: Option<String>,
}

impl VoiceView {
    /// The output level changes many times a second while Fermix speaks;
    /// everything else changes rarely. Only a real change redraws the page.
    pub fn same_apart_from_level(&self, other: &VoiceView) -> bool {
        let mut this = self.clone();
        this.level = other.level;
        this == *other
    }
}

pub struct VoicePage {
    pub root: adw::PreferencesPage,
    mascot: Mascot,
    pub companion: adw::SwitchRow,
    status_line: gtk::Box,
    status: gtk::Label,
    status_icon: gtk::Image,
    call: gtk::Button,
    mute: gtk::ToggleButton,
    stop: gtk::Button,
    gate_group: adw::PreferencesGroup,
    gate_row: adw::ActionRow,
    gate_button: gtk::Button,
    live: adw::PreferencesGroup,
    caption: adw::ActionRow,
    task: adw::ActionRow,
    cancel_task: gtk::Button,
    usage: adw::ActionRow,
    /// What was drawn last, so a new output level alone redraws only the mascot.
    shown: RefCell<Option<VoiceView>>,
}

impl VoicePage {
    pub fn new() -> VoicePage {
        let mascot = Mascot::new(200, 176);
        let (call_group, status_line, status, status_icon) = call_group(&mascot);
        let (call, mute, stop) = controls();
        call_group.add(&controls_bar(&call, &mute, &stop));
        let gate_row = adw::ActionRow::new();
        let gate_button = gtk::Button::builder()
            .valign(gtk::Align::Center)
            .css_classes(["suggested-action"])
            .build();
        gate_button.set_action_name(Some("win.voice-fix"));
        gate_row.add_suffix(&gate_button);
        let gate_group = adw::PreferencesGroup::new();
        gate_group.add(&gate_row);
        let (live, caption, task, usage) = live_group();
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
        let root = adw::PreferencesPage::new();
        root.add(&statement_group());
        // What stands in the way comes before the controls it blocks (spec §4.3).
        for group in [&gate_group, &call_group, &live] {
            root.add(group);
        }
        root.add(&more_group(&companion));
        VoicePage {
            root,
            mascot,
            companion,
            status_line,
            status,
            status_icon,
            call,
            mute,
            stop,
            gate_group,
            gate_row,
            gate_button,
            live,
            caption,
            task,
            cancel_task,
            usage,
            shown: RefCell::default(),
        }
    }

    pub fn render(&self, view: &VoiceView) {
        self.mascot.set_level(view.level);
        let unchanged = self
            .shown
            .borrow()
            .as_ref()
            .is_some_and(|last| last.same_apart_from_level(view));
        if unchanged {
            return;
        }
        *self.shown.borrow_mut() = Some(view.clone());
        self.mascot.set_expression(view.expression);
        self.mascot.set_in_call(view.in_call);
        let gate_shown = !view.gate.ready && !view.in_call;
        self.render_status(view, !gate_shown);
        self.render_controls(view);
        self.render_gate(&view.gate, gate_shown);
        self.render_live(view);
    }

    /// The word under the mascot. While the gate row says what is in the way,
    /// the word would only repeat it, so it steps aside.
    fn render_status(&self, view: &VoiceView, shown: bool) {
        self.status_line.set_visible(shown);
        self.status.set_text(&view.word);
        // A word is a heading; the error mode's sentence reads as body text.
        if view.palette == Palette::Error {
            self.status.remove_css_class("title-4");
        } else {
            self.status.add_css_class("title-4");
        }
        self.status_icon.set_icon_name(Some(view.icon));
        let tone = tone(view.palette);
        for widget in [
            self.status.upcast_ref::<gtk::Widget>(),
            self.status_icon.upcast_ref(),
        ] {
            for class in TONES {
                widget.remove_css_class(class);
            }
            if let Some(class) = tone {
                widget.add_css_class(class);
            }
        }
    }

    fn render_controls(&self, view: &VoiceView) {
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

    fn render_gate(&self, gate: &VoiceGate, shown: bool) {
        self.gate_group.set_visible(shown);
        self.gate_row
            .set_title(&glib::markup_escape_text(&gate.sentence));
        let verb = gate.action.map(action_verb);
        self.gate_button.set_visible(verb.is_some());
        if let Some(verb) = verb {
            self.gate_button.set_label(verb);
        }
    }

    fn render_live(&self, view: &VoiceView) {
        let rows = [
            (&self.caption, &view.caption),
            (&self.task, &view.task),
            (&self.usage, &view.usage),
        ];
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

/// The companion window's honest footer (spec_voice §2.4).
const COMPANION_NOTE: &str = "A small window you can leave open beside your work. Fermix cannot \
    keep it above other windows on Linux; GNOME can: press Alt+Space and choose Always on Top.";

/// Adwaita's status colours; each mode also has its word and icon.
const TONES: [&str; 4] = ["accent", "warning", "success", "error"];

fn tone(palette: Palette) -> Option<&'static str> {
    match palette {
        Palette::Accent => Some("accent"),
        Palette::Warning => Some("warning"),
        Palette::Success => Some("success"),
        Palette::Error => Some("error"),
        Palette::Faint | Palette::Secondary => None,
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

/// M38 §6.5, verbatim and above every control that opens the microphone.
fn statement_group() -> adw::PreferencesGroup {
    let label = gtk::Label::builder()
        .label(MICROPHONE_STATEMENT)
        .selectable(true)
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();
    let group = adw::PreferencesGroup::new();
    group.add(&label);
    group
}

fn call_group(mascot: &Mascot) -> (adw::PreferencesGroup, gtk::Box, gtk::Label, gtk::Image) {
    let stage = gtk::Box::builder()
        .halign(gtk::Align::Center)
        .margin_top(12)
        .build();
    stage.append(&mascot.widget);
    let status_icon = gtk::Image::new();
    // In the error mode the word is a sentence.
    let status = gtk::Label::builder()
        .css_classes(["title-4"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .max_width_chars(40)
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
    (group, line, status, status_icon)
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
        .tooltip_text("Stop the reply")
        .css_classes(["circular"])
        .valign(gtk::Align::Center)
        .build();
    stop.set_action_name(Some("app.voice-stop"));
    (call, mute, stop)
}

fn controls_bar(call: &gtk::Button, mute: &gtk::ToggleButton, stop: &gtk::Button) -> gtk::Box {
    let bar = gtk::Box::builder()
        .spacing(12)
        .halign(gtk::Align::Center)
        .margin_top(12)
        .margin_bottom(6)
        .build();
    bar.append(mute);
    bar.append(call);
    bar.append(stop);
    bar
}

fn live_group() -> (
    adw::PreferencesGroup,
    adw::ActionRow,
    adw::ActionRow,
    adw::ActionRow,
) {
    // Property rows: a small title, the value below it.
    let row = |title: &str| {
        adw::ActionRow::builder()
            .title(title)
            .css_classes(["property"])
            .subtitle_selectable(true)
            .visible(false)
            .build()
    };
    let (caption, task, usage) = (row("Last said"), row("Task"), row("Voice so far"));
    let group = adw::PreferencesGroup::builder()
        .title("This call")
        .visible(false)
        .build();
    for r in [&caption, &task, &usage] {
        group.add(r);
    }
    (group, caption, task, usage)
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
