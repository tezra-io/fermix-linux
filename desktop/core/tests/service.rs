use fermix_client::service::{
    background_switch, disable_warning, login_answer, service_line, LoginAnswer, ServiceRead,
    UnitFacts, BACKGROUND_NOTE, VENDOR_UNIT,
};

fn unit(file_state: &str, active: &str) -> UnitFacts {
    UnitFacts {
        load_state: "loaded".into(),
        unit_file_state: file_state.into(),
        active_state: active.into(),
        fragment_path: VENDOR_UNIT.into(),
    }
}

fn read(file_state: &str, active: &str, linger: Option<bool>) -> ServiceRead {
    ServiceRead::Unit {
        unit: unit(file_state, active),
        linger,
    }
}

#[test]
fn the_service_line_says_registration_run_state_and_logout_in_that_order() {
    let line = service_line(&read("enabled", "active", Some(true)));
    assert_eq!(line, "Enabled · running · stays on after logout");
    let line = service_line(&read("disabled", "inactive", Some(false)));
    assert_eq!(line, "Disabled · stopped · stops when you log out");
}

#[test]
fn an_unread_linger_is_left_out_rather_than_guessed() {
    assert_eq!(
        service_line(&read("enabled", "failed", None)),
        "Enabled · failed"
    );
    assert_eq!(
        service_line(&read("enabled", "activating", None)),
        "Enabled · starting"
    );
}

#[test]
fn an_unreachable_manager_is_not_a_stopped_service() {
    assert_eq!(
        service_line(&ServiceRead::Unreachable),
        "Unavailable (service manager)"
    );
    let missing = ServiceRead::Unit {
        unit: UnitFacts {
            load_state: "not-found".into(),
            ..unit("", "inactive")
        },
        linger: Some(true),
    };
    assert_eq!(service_line(&missing), "Not installed");
}

#[test]
fn the_switch_follows_the_unit_file_state_and_offers_itself_when_bound() {
    let on = background_switch(Some(&read("enabled", "active", Some(true))), true, false);
    assert!(on.on && on.sensitive);
    assert_eq!(on.note, BACKGROUND_NOTE);
    let off = background_switch(Some(&read("disabled", "inactive", Some(true))), true, false);
    assert!(!off.on && off.sensitive);
}

#[test]
fn an_unbound_disabled_service_names_the_install_command_and_stays_off() {
    let switch = background_switch(Some(&read("disabled", "inactive", None)), false, false);
    assert!(!switch.on && !switch.sensitive);
    assert!(
        switch.note.contains("fermix service install"),
        "{}",
        switch.note
    );
}

#[test]
fn a_shadowing_unit_is_refused_and_named() {
    let shadowed = ServiceRead::Unit {
        unit: UnitFacts {
            fragment_path: "/home/me/.config/systemd/user/fermix.service".into(),
            ..unit("enabled", "active")
        },
        linger: Some(true),
    };
    let switch = background_switch(Some(&shadowed), true, false);
    assert!(switch.on && !switch.sensitive);
    assert!(
        switch.note.contains("fermix service install"),
        "{}",
        switch.note
    );
}

#[test]
fn the_switch_waits_while_unread_unreachable_missing_or_busy() {
    assert!(!background_switch(None, true, false).sensitive);
    let unreachable = background_switch(Some(&ServiceRead::Unreachable), true, false);
    assert!(!unreachable.sensitive);
    assert!(
        unreachable.note.contains("service manager"),
        "{}",
        unreachable.note
    );
    let busy = background_switch(Some(&read("enabled", "active", Some(true))), true, true);
    assert!(busy.on && !busy.sensitive);
}

#[test]
fn the_disable_warning_counts_what_it_interrupts_and_never_invents_zero() {
    let none = disable_warning(Some(0));
    assert!(!none.contains("in progress"), "{none}");
    assert!(disable_warning(Some(1)).contains("1 conversation is in progress"));
    assert!(disable_warning(Some(3)).contains("3 conversations are in progress"));
    assert!(disable_warning(None).contains("could not say"));
}

#[test]
fn the_portal_answer_is_kept_only_when_granted() {
    assert_eq!(login_answer(0, Some(true), true), LoginAnswer::Set(true));
    assert_eq!(login_answer(0, Some(false), false), LoginAnswer::Set(false));
    assert_eq!(login_answer(1, None, true), LoginAnswer::Cancelled);
    assert!(matches!(
        login_answer(0, Some(false), true),
        LoginAnswer::Refused(_)
    ));
    assert!(matches!(
        login_answer(2, None, true),
        LoginAnswer::Refused(_)
    ));
}

#[test]
fn an_enabled_unit_that_stops_at_logout_is_not_in_the_background() {
    let lingerless = background_switch(Some(&read("enabled", "active", Some(false))), true, false);
    assert!(!lingerless.on && lingerless.sensitive);
    let unknown = background_switch(Some(&read("enabled", "active", None)), true, false);
    assert!(unknown.on, "an unread linger does not turn the switch off");
}
