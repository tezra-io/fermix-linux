//! Copying a secret (M60 cues §4.4): a token, an address with a credential in it, or the phone
//! dialog's pairing link goes on the clipboard with the hint a clipboard manager that honours it
//! reads as "do not keep this", and is taken back when it leaves the view or the view goes, if
//! the clipboard still holds this copy.

use adw::prelude::*;
use gtk::{gdk, glib};
use std::cell::RefCell;

const SECRET_HINT: (&str, &[u8]) = ("x-kde-passwordManagerHint", b"secret");

/// One copy a view made, so the view can take it back as it goes.
#[derive(Default)]
pub struct SecretCopy {
    offered: RefCell<Option<Offer>>,
}

struct Offer {
    clipboard: gdk::Clipboard,
    provider: gdk::ContentProvider,
    /// What it is, for the log, never its value.
    what: &'static str,
}

impl SecretCopy {
    /// Puts `text` on `clipboard` with the secret hint, replacing this view's last copy. `what`
    /// names it for the log, never its value; a copy that failed is logged there. An empty
    /// `text` (the daemon handed out nothing) is logged and not copied: a caller that draws a
    /// sentence for it checks first.
    pub fn copy(&self, clipboard: &gdk::Clipboard, text: &str, what: &'static str) {
        self.take_back();
        if text.is_empty() {
            glib::g_warning!("fermix", "{what} was not copied: Fermix handed out nothing");
            return;
        }
        let (mime, hint) = SECRET_HINT;
        let provider = gdk::ContentProvider::new_union(&[
            gdk::ContentProvider::for_value(&text.to_value()),
            gdk::ContentProvider::for_bytes(mime, &glib::Bytes::from_static(hint)),
        ]);
        if let Err(e) = clipboard.set_content(Some(&provider)) {
            glib::g_warning!("fermix", "{what} was not copied: {e}");
            return;
        }
        *self.offered.borrow_mut() = Some(Offer {
            clipboard: clipboard.clone(),
            provider,
            what,
        });
    }

    /// Takes this view's copy off the clipboard, if the clipboard still holds it.
    pub fn take_back(&self) {
        let Some(offer) = self.offered.borrow_mut().take() else {
            return;
        };
        let clipboard = &offer.clipboard;
        if !clipboard.is_local() || clipboard.content().as_ref() != Some(&offer.provider) {
            return;
        }
        if let Err(e) = clipboard.set_content(None::<&gdk::ContentProvider>) {
            let what = offer.what;
            glib::g_warning!("fermix", "{what} was not taken off the clipboard: {e}");
        }
    }
}

/// A view that goes takes its copy with it.
impl Drop for SecretCopy {
    fn drop(&mut self) {
        self.take_back();
    }
}
