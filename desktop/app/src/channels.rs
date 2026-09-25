//! Channels: one row per channel with its state in words, a switch, and a way
//! into its credentials (M38 §5.7). The daemon supplies every field of a
//! channel; this pane only knows that a channel's switch is its section's
//! toggle row, the last one.

use crate::descriptor::SectionView;
use crate::marks;
use crate::settings::SettingsData;
use adw::prelude::*;
use fermix_client::model::ChannelRow;
use fermix_client::settings::{channel_word, sections_for, Kind, Section};
use gtk::glib::{self, variant::ToVariant};
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;

/// One channel line as drawn; the row is rebuilt only when this changes.
#[derive(Debug, Clone, PartialEq)]
struct Line {
    name: String,
    title: String,
    word: &'static str,
    configured: bool,
    /// The switch's row key and value, once the channel's section has been read.
    switch: Option<(String, bool)>,
    error: Option<String>,
    locked: bool,
}

pub struct ChannelsPane {
    pub page: adw::PreferencesPage,
    list: adw::PreferencesGroup,
    rows: RefCell<Vec<adw::ActionRow>>,
    shown: RefCell<Vec<Line>>,
}

impl ChannelsPane {
    pub fn new() -> ChannelsPane {
        let list = adw::PreferencesGroup::builder()
            .description("Where you can reach Fermix. A channel starts working once it is turned on and set up.")
            .build();
        let page = adw::PreferencesPage::new();
        page.add(&list);
        ChannelsPane {
            page,
            list,
            rows: RefCell::default(),
            shown: RefCell::default(),
        }
    }

    /// The editor-connection section, drawn under the channel list.
    pub fn editors_view(&self, sections: &[Section]) -> Rc<SectionView> {
        let editors = sections_for("channels", sections)
            .into_iter()
            .find(|s| !s.id.starts_with("channels."));
        let (id, title) = editors.map_or(("editors", "Editors"), |s| (&s.id, &s.title));
        let view = SectionView::new(id, Some(title));
        self.page.add(&view.group);
        view
    }

    pub fn render(&self, channels: &[ChannelRow], data: &SettingsData, locked: bool) {
        let lines: Vec<Line> = channels.iter().map(|c| line(c, data, locked)).collect();
        if *self.shown.borrow() == lines {
            return;
        }
        for old in self.rows.borrow_mut().drain(..) {
            self.list.remove(&old);
        }
        let fresh: Vec<adw::ActionRow> = lines.iter().map(channel_row).collect();
        for row in &fresh {
            self.list.add(row);
        }
        *self.rows.borrow_mut() = fresh;
        *self.shown.borrow_mut() = lines;
    }
}

pub fn section_id(channel: &str) -> String {
    format!("channels.{channel}")
}

/// The channel's switch is the last toggle row of its section.
pub fn switch_key(data: &SettingsData, channel: &str) -> Option<String> {
    let rows = data.rows.get(&section_id(channel))?;
    rows.rows
        .iter()
        .rev()
        .find(|r| r.kind == Kind::Toggle)
        .map(|r| r.key.clone())
}

fn line(channel: &ChannelRow, data: &SettingsData, locked: bool) -> Line {
    let section = section_id(&channel.name);
    let title = data
        .sections
        .iter()
        .flatten()
        .find(|s| s.id == section)
        .map_or_else(|| channel.name.clone(), |s| s.title.clone());
    let key = switch_key(data, &channel.name);
    let switch = key.as_ref().and_then(|key| {
        let row = data
            .rows
            .get(&section)?
            .rows
            .iter()
            .find(|r| &r.key == key)?;
        Some((key.clone(), row.value.as_bool().unwrap_or(false)))
    });
    let error = key.and_then(|key| data.errors.get(&(section, key)).cloned());
    Line {
        name: channel.name.clone(),
        title,
        word: channel_word(channel.enabled, channel.status.as_deref()),
        configured: channel.configured,
        switch,
        error,
        locked,
    }
}

fn channel_row(line: &Line) -> adw::ActionRow {
    let subtitle = line.error.as_deref().unwrap_or(line.word);
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&line.title))
        .subtitle(glib::markup_escape_text(subtitle))
        .build();
    row.add_prefix(&marks::mark(marks::Kind::Channel, &line.name));
    if line.error.is_some() {
        row.add_css_class("setting-refused");
    }
    let verb = if line.configured {
        "Change…"
    } else {
        "Set up…"
    };
    let setup = gtk::Button::builder()
        .label(verb)
        .valign(gtk::Align::Center)
        .build();
    setup.set_action_name(Some("win.channel-setup"));
    setup.set_action_target_value(Some(&line.name.to_variant()));
    setup.update_property(&[gtk::accessible::Property::Label(&format!(
        "{verb} {}",
        line.title
    ))]);
    row.add_suffix(&setup);
    if let Some((key, on)) = &line.switch {
        row.add_suffix(&channel_switch(line, key, *on));
    }
    row
}

fn channel_switch(line: &Line, key: &str, on: bool) -> gtk::Switch {
    let switch = gtk::Switch::builder()
        .active(on)
        .valign(gtk::Align::Center)
        .sensitive(!line.locked)
        .build();
    switch.update_property(&[gtk::accessible::Property::Label(&format!(
        "Turn {} on or off",
        line.title
    ))]);
    let (section, key) = (section_id(&line.name), key.to_owned());
    switch.connect_active_notify(move |switch| {
        let value = Value::Bool(switch.is_active()).to_string();
        let target = (section.as_str(), key.as_str(), value).to_variant();
        if let Err(e) = switch.activate_action("win.setting-apply", Some(&target)) {
            glib::g_warning!("fermix", "a channel switch could not be sent: {e}");
        }
    });
    switch
}
