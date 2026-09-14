//! The model picker.
//!
//! One listing, paged by the cursor the daemon returns, searched by the query
//! the daemon takes. The rows are whatever `providers.models.list` answered and
//! their words are the daemon's.
//!
//! A live listing never degrades to the catalog. The two answer different
//! questions: the catalog is what this build ships, the live listing is what
//! the provider has right now, and a live fetch that fails renders the daemon's
//! own sentence and leaves the rows that were already there alone. Nothing is
//! swapped underneath a person to keep the dialog looking full.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::management::types::ProviderModel;
use crate::metrics;
use crate::models::providers::ProvidersModel;
use crate::models::spawn;
use crate::ui::CaptionRow;

/// The least of a listing the picker shows before it scrolls: six rows.
///
/// A dialog sized only by its content opens at the height of whatever had
/// arrived when it was presented, and this listing is fetched after the dialog
/// is on screen: the reference capture shows one row and a half of it. This is
/// a floor rather than a size. The dialog still grows with its content and
/// still stops at the height of the window it is presented in.
const LISTING_FLOOR: i32 = 6 * metrics::ROW_MINIMUM_HEIGHT;

/// What one chosen model is handed to.
type Chosen = Box<dyn Fn(&str)>;

/// The picker.
pub struct ModelPicker {
    providers: Rc<ProvidersModel>,
    provider: String,
    dialog: adw::Dialog,
    list: gtk::ListBox,
    search: gtk::SearchEntry,
    more: gtk::Button,
    live: gtk::Button,
    notice: CaptionRow,
    /// The models on screen, in the order they were answered.
    models: RefCell<Vec<ProviderModel>>,
    /// The page after the ones on screen, where the daemon named one.
    cursor: RefCell<Option<String>>,
    chosen: RefCell<Option<Chosen>>,
}

impl ModelPicker {
    /// Open the picker over one provider.
    pub fn present(
        providers: Rc<ProvidersModel>,
        provider: &str,
        parent: &impl IsA<gtk::Widget>,
        chosen: impl Fn(&str) + 'static,
    ) {
        let picker = Rc::new(Self::build(providers, provider));
        picker.chosen.replace(Some(Box::new(chosen)));
        picker.connect();
        picker.dialog.present(Some(parent.as_ref()));
        picker.search.grab_focus();
        picker.reload(None, false);
    }

    fn build(providers: Rc<ProvidersModel>, provider: &str) -> Self {
        let search = gtk::SearchEntry::builder()
            .placeholder_text(copy::text(Key::ProviderModelsSearch))
            .hexpand(true)
            .build();
        search.update_property(&[gtk::accessible::Property::Label(&copy::text(
            Key::ProviderModelsSearch,
        ))]);

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .build();
        list.add_css_class("boxed-list");

        let more = gtk::Button::builder()
            .label(copy::text(Key::ProviderModelsMore))
            .halign(gtk::Align::Center)
            .visible(false)
            .build();
        let live = gtk::Button::builder()
            .label(copy::text(Key::ProviderModelsLive))
            .halign(gtk::Align::Center)
            .build();

        let notice = CaptionRow::new();
        let notice_group = crate::ui::caption_group(&notice);

        let column = crate::ui::column();
        column.append(&search);
        column.append(&list);
        column.append(&notice_group);
        column.append(&more);
        column.append(&live);
        column.set_margin_top(metrics::SPACE_GUTTER);
        column.set_margin_bottom(metrics::SPACE_GUTTER);
        column.set_margin_start(metrics::SPACE_GUTTER);
        column.set_margin_end(metrics::SPACE_GUTTER);

        let scroller = crate::ui::scrolled(&column);
        scroller.set_propagate_natural_height(true);
        scroller.set_min_content_height(LISTING_FLOOR);

        let toolbar = adw::ToolbarView::builder().content(&scroller).build();
        toolbar.add_top_bar(
            &adw::HeaderBar::builder()
                .title_widget(&adw::WindowTitle::new(
                    &copy::text(Key::ProviderModelDialogTitle),
                    "",
                ))
                .build(),
        );

        let dialog = adw::Dialog::builder()
            .title(copy::text(Key::ProviderModelDialogTitle))
            .content_width(metrics::CLAMP_MAXIMUM)
            .child(&toolbar)
            .build();

        Self {
            providers,
            provider: provider.to_string(),
            dialog,
            list,
            search,
            more,
            live,
            notice,
            models: RefCell::new(Vec::new()),
            cursor: RefCell::new(None),
            chosen: RefCell::new(None),
        }
    }

    fn connect(self: &Rc<Self>) {
        {
            let picker = Rc::clone(self);
            self.search.connect_search_changed(move |entry| {
                let query = entry.text().to_string();
                picker.models.borrow_mut().clear();
                picker.reload(Some(query), false);
            });
        }
        {
            let picker = Rc::clone(self);
            self.more.connect_clicked(move |_| {
                let cursor = picker.cursor.borrow().clone();
                let query = picker.query();
                picker.page(query, cursor, false);
            });
        }
        {
            let picker = Rc::clone(self);
            self.live.connect_clicked(move |_| {
                picker.models.borrow_mut().clear();
                picker.reload(picker.query(), true);
            });
        }
    }

    fn query(&self) -> Option<String> {
        let text = self.search.text().trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    /// Read the first page.
    fn reload(self: &Rc<Self>, query: Option<String>, live: bool) {
        self.cursor.replace(None);
        self.page(query, None, live);
    }

    /// Read one page and add it to what is on screen.
    fn page(self: &Rc<Self>, query: Option<String>, cursor: Option<String>, live: bool) {
        let picker = Rc::clone(self);
        let provider = self.provider.clone();
        let providers = Rc::clone(&self.providers);

        spawn(async move {
            match providers.models(&provider, query, cursor, live).await {
                Ok(page) => {
                    picker.models.borrow_mut().extend(page.models);
                    picker.cursor.replace(page.cursor.clone());
                    picker.notice.set(None);
                    picker.draw();
                }
                // The daemon's own sentence for a listing it could not take.
                // The rows already on screen stay where they are: they came
                // from a different question and are still the answer to it.
                Err(sentence) => picker.notice.set(Some(&sentence.text)),
            }
        });
    }

    fn draw(self: &Rc<Self>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }

        for model in self.models.borrow().iter() {
            self.list.append(&self.row(model));
        }

        self.more.set_visible(self.cursor.borrow().is_some());
    }

    fn row(self: &Rc<Self>, model: &ProviderModel) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(model.label.as_str())
            .subtitle(model.id.as_str())
            .activatable(true)
            .build();

        let picker = Rc::clone(self);
        let id = model.id.clone();
        row.connect_activated(move |_| {
            if let Some(chosen) = picker.chosen.borrow().as_ref() {
                chosen(&id);
            }
            picker.dialog.close();
        });

        row
    }
}
