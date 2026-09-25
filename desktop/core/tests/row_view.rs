//! What a provider row says and offers, in every state the design names
//! (design_final §3). The GTK side only draws what this returns.

use fermix_client::model::ProviderRow;
use fermix_client::providers::{Door, ImportSource};
use fermix_client::view::{row_view, Activity, CopyLink, Lead, MenuItem, Recent, Suffix};
use serde_json::json;

fn row(id: &str, auth_modes: &[&str], auth_mode: &str) -> ProviderRow {
    serde_json::from_value(json!({
        "id": id, "label": id, "auth_modes": auth_modes, "auth_mode": auth_mode,
        "configured": false, "primary": false, "present_key": false, "default_model": null,
        "reasoning_effort": null, "fast": null, "account_label": null, "token_state": null
    }))
    .unwrap()
}

fn codex() -> ProviderRow {
    row("openai_codex", &["oauth"], "oauth")
}

fn anthropic() -> ProviderRow {
    row("anthropic", &["api_key", "oauth"], "oauth")
}

fn signed_in(mut r: ProviderRow) -> ProviderRow {
    r.configured = true;
    r.token_state = Some("valid".into());
    r
}

/// A resting row: its one visible button, then the rest behind the ⋮ menu.
fn resting(lead: Option<(Door, &str)>, more: Vec<MenuItem>) -> Suffix {
    Suffix::Resting {
        lead: lead.map(|(door, verb)| Lead {
            door,
            verb: verb.into(),
        }),
        more,
    }
}

fn job(door: Door, phase: &str) -> Activity {
    Activity::Job {
        job_id: "job:1".into(),
        door,
        phase: Some(phase.into()),
        has_link: door == Door::BrowserSignIn,
        browser_failed: false,
    }
}

#[test]
fn chatgpt_not_signed_in_leads_with_the_browser() {
    let v = row_view(&codex(), &Activity::Idle, None);
    assert_eq!(v.subtitle, "Not signed in");
    assert!(!v.is_error);
    assert_eq!(
        v.suffix,
        resting(
            Some((Door::BrowserSignIn, "Sign in")),
            vec![MenuItem::Door(Door::Import(ImportSource::CodexCli))]
        )
    );
}

#[test]
fn anthropic_not_signed_in_leads_with_claude_code_and_never_the_browser() {
    let v = row_view(&anthropic(), &Activity::Idle, None);
    assert_eq!(
        v.suffix,
        resting(
            Some((
                Door::Import(ImportSource::ClaudeCode),
                "Import from Claude Code"
            )),
            vec![
                MenuItem::Door(Door::SetupToken),
                MenuItem::Door(Door::ApiKey)
            ]
        )
    );
}

#[test]
fn a_key_only_provider_without_a_key_offers_add_key() {
    let v = row_view(
        &row("openai", &["api_key"], "api_key"),
        &Activity::Idle,
        None,
    );
    assert_eq!(v.subtitle, "No API key");
    assert_eq!(v.suffix, resting(Some((Door::ApiKey, "Add key…")), vec![]));
}

#[test]
fn a_stored_key_reads_key_added_with_its_menu() {
    let mut r = row("openai", &["api_key"], "api_key");
    r.configured = true;
    r.present_key = true;
    let v = row_view(&r, &Activity::Idle, None);
    assert_eq!(v.subtitle, "Key added");
    assert_eq!(
        v.suffix,
        resting(
            None,
            vec![
                MenuItem::MakePrimary,
                MenuItem::ReplaceKey,
                MenuItem::RemoveKey
            ]
        )
    );
}

#[test]
fn the_primary_row_says_so_and_does_not_offer_make_primary() {
    let mut r = signed_in(codex());
    r.primary = true;
    let v = row_view(&r, &Activity::Idle, None);
    assert_eq!(v.subtitle, "Signed in · Primary");
    assert_eq!(
        v.suffix,
        resting(
            Some((Door::BrowserSignIn, "Sign in again")),
            vec![
                MenuItem::Door(Door::Import(ImportSource::CodexCli)),
                MenuItem::SignOut
            ]
        )
    );
}

/// Sign-in is the primary way in, so a signed-in row keeps it in view; only
/// the rest waits behind ⋮ (owner, 2026-09-25).
#[test]
fn a_signed_in_row_keeps_its_sign_in_visible_and_the_rest_in_the_menu() {
    let v = row_view(&signed_in(codex()), &Activity::Idle, None);
    assert_eq!(
        v.suffix,
        resting(
            Some((Door::BrowserSignIn, "Sign in again")),
            vec![
                MenuItem::MakePrimary,
                MenuItem::Door(Door::Import(ImportSource::CodexCli)),
                MenuItem::SignOut
            ]
        )
    );
}

#[test]
fn a_provider_on_a_key_still_leads_with_its_sign_in() {
    let mut r = row("xai", &["api_key", "oauth"], "api_key");
    r.configured = true;
    r.present_key = true;
    let v = row_view(&r, &Activity::Idle, None);
    assert_eq!(
        v.suffix,
        resting(
            Some((Door::BrowserSignIn, "Sign in")),
            vec![
                MenuItem::MakePrimary,
                MenuItem::ReplaceKey,
                MenuItem::RemoveKey
            ]
        )
    );
}

#[test]
fn an_expired_sign_in_asks_to_sign_in_again() {
    let mut r = signed_in(anthropic());
    r.token_state = Some("expired".into());
    let v = row_view(&r, &Activity::Idle, None);
    assert_eq!(v.subtitle, "Sign-in expired");
    assert!(matches!(
        v.suffix,
        Suffix::Resting {
            lead: Some(Lead {
                door: Door::Import(ImportSource::ClaudeCode),
                ..
            }),
            ..
        }
    ));
    let mut chatgpt = signed_in(codex());
    chatgpt.token_state = Some("revoked".into());
    assert_eq!(
        row_view(&chatgpt, &Activity::Idle, None).suffix,
        resting(
            Some((Door::BrowserSignIn, "Sign in again")),
            vec![MenuItem::Door(Door::Import(ImportSource::CodexCli))]
        )
    );
}

#[test]
fn waiting_for_the_browser_offers_copy_link_and_cancel() {
    let v = row_view(
        &codex(),
        &job(Door::BrowserSignIn, "awaiting_browser"),
        None,
    );
    assert_eq!(v.subtitle, "Continue in your browser");
    assert_eq!(
        v.suffix,
        Suffix::Busy {
            copy_link: CopyLink::Offered,
            cancel: true
        }
    );
}

#[test]
fn a_browser_that_did_not_open_says_so_and_keeps_the_link() {
    let activity = Activity::Job {
        job_id: "job:1".into(),
        door: Door::BrowserSignIn,
        phase: Some("awaiting_browser".into()),
        has_link: true,
        browser_failed: true,
    };
    let v = row_view(&codex(), &activity, None);
    assert_eq!(v.subtitle, "Your browser did not open");
    assert_eq!(
        v.suffix,
        Suffix::Busy {
            copy_link: CopyLink::Rescue,
            cancel: true
        }
    );
}

#[test]
fn verifying_is_a_spinner_with_nothing_to_press() {
    let v = row_view(&codex(), &job(Door::BrowserSignIn, "verifying"), None);
    assert_eq!(v.subtitle, "Checking the sign-in");
    assert_eq!(
        v.suffix,
        Suffix::Busy {
            copy_link: CopyLink::None,
            cancel: false
        }
    );
}

#[test]
fn an_import_names_the_login_it_reads() {
    let v = row_view(
        &anthropic(),
        &job(Door::Import(ImportSource::ClaudeCode), "reading_keychain"),
        None,
    );
    assert_eq!(v.subtitle, "Reading your Claude Code login");
    assert_eq!(
        v.suffix,
        Suffix::Busy {
            copy_link: CopyLink::None,
            cancel: true
        }
    );
}

#[test]
fn a_failure_shows_the_daemons_sentence_as_an_error_and_the_door_again() {
    let v = row_view(
        &codex(),
        &Activity::Failed("The sign-in was declined.".into()),
        None,
    );
    assert_eq!(v.subtitle, "The sign-in was declined.");
    assert!(v.is_error);
    assert!(matches!(
        v.suffix,
        Suffix::Resting {
            lead: Some(Lead {
                door: Door::BrowserSignIn,
                ..
            }),
            ..
        }
    ));
}

#[test]
fn a_cancelled_sign_in_says_so() {
    let v = row_view(&codex(), &Activity::Cancelled, None);
    assert_eq!(v.subtitle, "Sign-in cancelled");
    assert!(!v.is_error);
}

#[test]
fn a_recent_sign_in_is_acknowledged_even_when_the_row_is_unchanged() {
    let mut r = signed_in(codex());
    r.primary = true;
    let before = row_view(&r, &Activity::Idle, None);
    let after = row_view(&r, &Activity::Idle, Some(Recent::SignedIn));
    assert_ne!(before.subtitle, after.subtitle);
    assert_eq!(after.subtitle, "Signed in just now · Primary");
}

#[test]
fn a_recent_claude_code_import_names_where_it_came_from() {
    let v = row_view(
        &signed_in(anthropic()),
        &Activity::Idle,
        Some(Recent::Imported(ImportSource::ClaudeCode)),
    );
    assert_eq!(v.subtitle, "Signed in just now with your Claude Code login");
}

#[test]
fn a_provider_that_needs_nothing_says_so() {
    let mut r = row("ollama", &["none"], "none");
    assert_eq!(
        row_view(&r, &Activity::Idle, None).suffix,
        resting(None, vec![])
    );
    r.configured = true;
    let v = row_view(&r, &Activity::Idle, None);
    assert_eq!(v.subtitle, "No sign-in needed");
    assert_eq!(v.suffix, resting(None, vec![MenuItem::MakePrimary]));
}

#[test]
fn saving_signing_out_and_switching_are_spinners() {
    for (activity, subtitle) in [
        (Activity::Saving, "Adding the key"),
        (Activity::SigningOut, "Signing out"),
        (Activity::RemovingKey, "Removing the key"),
        (Activity::Switching, "Switching"),
    ] {
        let v = row_view(&signed_in(codex()), &activity, None);
        assert_eq!(v.subtitle, subtitle);
        assert_eq!(
            v.suffix,
            Suffix::Busy {
                copy_link: CopyLink::None,
                cancel: false
            }
        );
    }
}

#[test]
fn every_door_has_a_verb_and_a_menu_label() {
    assert_eq!(Door::BrowserSignIn.verb(false), "Sign in");
    assert_eq!(Door::BrowserSignIn.verb(true), "Sign in again");
    assert_eq!(
        Door::Import(ImportSource::ClaudeCode).verb(false),
        "Import from Claude Code"
    );
    assert_eq!(
        Door::Import(ImportSource::CodexCli).verb(false),
        "Import from Codex CLI"
    );
    assert_eq!(Door::SetupToken.verb(false), "Paste a setup token…");
    assert_eq!(Door::ApiKey.verb(false), "Use an API key…");
}

#[test]
fn a_job_without_its_id_yet_offers_no_cancel_that_could_not_work() {
    for door in [Door::BrowserSignIn, Door::Import(ImportSource::ClaudeCode)] {
        let starting = Activity::Job {
            job_id: String::new(),
            door,
            phase: None,
            has_link: false,
            browser_failed: false,
        };
        let v = row_view(&anthropic(), &starting, None);
        assert!(
            matches!(v.suffix, Suffix::Busy { cancel: false, .. }),
            "{door:?}"
        );
    }
}

#[test]
fn a_sign_in_found_running_without_its_link_says_so_and_can_be_cancelled() {
    let adopted = Activity::Job {
        job_id: "job:9".into(),
        door: Door::BrowserSignIn,
        phase: Some("awaiting_browser".into()),
        has_link: false,
        browser_failed: false,
    };
    let v = row_view(&codex(), &adopted, None);
    assert_eq!(v.subtitle, "A sign-in is already running");
    assert_eq!(
        v.suffix,
        Suffix::Busy {
            copy_link: CopyLink::None,
            cancel: true
        }
    );
}

#[test]
fn just_now_is_dropped_once_the_row_is_no_longer_connected() {
    let v = row_view(&codex(), &Activity::Idle, Some(Recent::SignedIn));
    assert_eq!(v.subtitle, "Not signed in");
    let mut key = row("openai", &["api_key"], "api_key");
    key.configured = true;
    assert_eq!(
        row_view(&key, &Activity::Idle, Some(Recent::KeyAdded)).subtitle,
        "No API key"
    );
}

#[test]
fn only_work_in_progress_is_in_flight() {
    assert!(job(Door::BrowserSignIn, "verifying").in_flight());
    for busy in [
        Activity::Saving,
        Activity::SigningOut,
        Activity::RemovingKey,
        Activity::Switching,
    ] {
        assert!(busy.in_flight(), "{busy:?}");
    }
    for resting in [
        Activity::Idle,
        Activity::Cancelled,
        Activity::Failed("no".into()),
    ] {
        assert!(!resting.in_flight(), "{resting:?}");
    }
}
