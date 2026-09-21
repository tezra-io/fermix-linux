//! The model picker.
//!
//! One listing, paged by the cursor the daemon returns, searched by the query
//! the daemon takes. The rows are whatever `providers.models.list` answered and
//! their words are the daemon's.
//!
//! The daemon never degrades a live listing to the catalog: the two answer
//! different questions, the catalog being what this build ships and the live
//! listing what the provider has right now. Where a provider has no live
//! listing at all, this dialog asks the second question itself — a separate,
//! explicit request — and says on screen which listing is showing. A live
//! fetch that fails for any other reason renders the daemon's
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
use crate::models::settings_model::Sentence;
use crate::models::spawn;
use crate::ui::plain;
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
        let asked = query.clone();

        spawn(async move {
            let answer = providers
                .models(&provider, query.clone(), cursor, live)
                .await;

            // A live ask this provider has no live listing for is asked again
            // as the catalogue, because a refusal where a list of models
            // belongs helps nobody choose one.
            let (answer, built_in) = match &answer {
                Err(sentence) if falls_back_to_catalog(live, sentence) => {
                    (providers.models(&provider, query, None, false).await, true)
                }
                _ => (answer, false),
            };

            match answer {
                Ok(page) => {
                    picker.models.borrow_mut().extend(page.models);
                    picker.cursor.replace(page.cursor.clone());
                    let nothing = picker.models.borrow().is_empty();
                    let said = match built_in {
                        // Which listing is on screen is said plainly, so the
                        // catalogue is never mistaken for the provider's own.
                        true => Some(copy::text(Key::ProviderModelsBuiltIn)),
                        false => empty_listing(nothing, asked.as_deref()),
                    };
                    picker.notice.set(said.as_deref());
                    picker.draw();
                }
                // The daemon's own sentence for a listing it could not take.
                Err(sentence) => {
                    picker.notice.set(Some(&sentence.text));
                    // The live path empties the list before it asks, so the
                    // rows must be redrawn or the screen keeps showing models
                    // that are no longer in the list behind it.
                    picker.draw();
                }
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
        let row = plain(
            adw::ActionRow::builder()
                .title(model.label.as_str())
                .subtitle(model.id.as_str())
                .activatable(true)
                .build(),
        );

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

/// What to say when a listing came back carrying nothing.
///
/// An empty answer and an unasked question look identical on screen, so the
/// one arm that used to clear the notice and draw nothing now says which of
/// the two happened. The two cases are not the same thing to a person: a
/// filter that matched nothing leaves the models where they are, while a
/// provider that listed none is a fact about the provider. Saying the second
/// when the first is true would be the product lying about someone's account.
///
/// `None` when there is something on screen, which is the ordinary case.
fn empty_listing(nothing_listed: bool, query: Option<&str>) -> Option<String> {
    if !nothing_listed {
        return None;
    }

    Some(
        match query.map(str::trim).filter(|asked| !asked.is_empty()) {
            Some(asked) => copy::fill(Key::ProviderModelsNoMatch, &[("{query}", asked)]),
            None => copy::text(Key::ProviderModelsEmpty),
        },
    )
}

/// Whether a refused listing should be asked for again from the catalogue.
///
/// Only a live listing can be answered with `unavailable`: the catalogue is
/// compiled into the daemon and is always there. So a live ask that comes
/// back unavailable means this provider has no live listing at all — for
/// openai_codex it never had one — and the useful thing to do is ask the
/// question the daemon can answer rather than show the person a refusal where
/// a list of models should be.
///
/// This is not the daemon degrading a live listing to the catalogue, which
/// the protocol forbids. It is a second, explicit request, and the row above
/// the list says which listing is on screen.
///
/// The macOS sheet diverges: it asks `live: true` unconditionally
/// (FermixAppCore/Settings/Panes/ProviderSheets.swift:326) and renders the
/// refusal, so a provider with no live listing shows a sentence where its
/// models should be. Measured on the engine side: openai_codex has no live
/// listing and never will, so that sheet can only ever refuse for it. The
/// divergence is deliberate and this side is the corrected one.
fn falls_back_to_catalog(asked_live: bool, sentence: &Sentence) -> bool {
    asked_live && sentence.code.as_deref() == Some(UNAVAILABLE)
}

/// The refusal code a daemon answers a live listing it cannot make with.
const UNAVAILABLE: &str = "unavailable";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_with_rows_in_it_says_nothing() {
        assert_eq!(empty_listing(false, None), None);
        assert_eq!(empty_listing(false, Some("gpt")), None);
    }

    #[test]
    fn an_empty_listing_with_no_filter_speaks_of_the_provider() {
        assert_eq!(
            empty_listing(true, None),
            Some(copy::text(Key::ProviderModelsEmpty))
        );
    }

    #[test]
    fn an_empty_listing_under_a_filter_speaks_of_the_filter() {
        let said = empty_listing(true, Some("gpt")).expect("an empty listing says something");
        assert!(said.contains("gpt"), "the filter is quoted back: {said}");
        assert_ne!(
            said,
            copy::text(Key::ProviderModelsEmpty),
            "a filter that matched nothing must not be reported as the provider having nothing"
        );
    }

    #[test]
    fn a_blank_filter_is_not_a_filter() {
        // A search box emptied by hand arrives as Some(""). Treating that as
        // a filter would quote an empty string back at the person.
        assert_eq!(
            empty_listing(true, Some("   ")),
            Some(copy::text(Key::ProviderModelsEmpty))
        );
    }
    fn refusal(code: Option<&str>) -> Sentence {
        Sentence {
            code: code.map(str::to_string),
            text: "the daemon said something".to_string(),
            reason: None,
        }
    }

    #[test]
    fn an_unavailable_live_listing_asks_the_catalogue_instead() {
        assert!(falls_back_to_catalog(true, &refusal(Some(UNAVAILABLE))));
    }

    #[test]
    fn a_refused_catalogue_listing_is_not_asked_again() {
        // The catalogue is compiled into the daemon. If it refuses that, a
        // second ask would refuse identically and the loop would be endless.
        assert!(!falls_back_to_catalog(false, &refusal(Some(UNAVAILABLE))));
    }

    #[test]
    fn any_other_refusal_stands_as_the_daemon_worded_it() {
        for code in ["invalid_params", "busy", "internal_error"] {
            assert!(
                !falls_back_to_catalog(true, &refusal(Some(code))),
                "{code} was treated as an absent live listing"
            );
        }
        assert!(!falls_back_to_catalog(true, &refusal(None)));
    }
}
