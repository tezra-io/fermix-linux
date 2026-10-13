use fermix_client::capped::Ended;
use fermix_client::service::{
    background_switch, binding_path, disable_warning, install_answer, install_not_run,
    install_outcome, install_timed_out, read_binding, service_line, start_command, Binding,
    ServiceRead, UnitFacts, BACKGROUND_NOTE, INSTALL_ARGV, INSTALL_CAP, INSTALL_COMMAND,
    INSTALL_UNREADABLE, SETUP_NOTE, VENDOR_UNIT,
};
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

const HOME: &str = "/home/someone";

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
fn an_unbound_service_offers_to_set_fermix_up() {
    let switch = background_switch(Some(&read("disabled", "inactive", None)), false, false);
    assert!(!switch.on && switch.sensitive);
    assert_eq!(switch.note, SETUP_NOTE);
    assert!(!SETUP_NOTE.contains("terminal"), "{SETUP_NOTE}");
    let busy = background_switch(Some(&read("disabled", "inactive", None)), false, true);
    assert!(!busy.on && !busy.sensitive);
}

#[test]
fn an_unbound_unit_that_is_already_on_is_not_offered_set_up() {
    let switch = background_switch(Some(&read("enabled", "active", Some(true))), false, false);
    assert!(switch.on && switch.sensitive);
    assert_eq!(switch.note, BACKGROUND_NOTE);
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
    for bound in [true, false] {
        let switch = background_switch(Some(&shadowed), bound, false);
        assert!(switch.on && !switch.sensitive);
        assert!(
            switch.note.contains("fermix service install"),
            "{}",
            switch.note
        );
    }
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
fn an_enabled_unit_that_stops_at_logout_is_not_in_the_background() {
    let lingerless = background_switch(Some(&read("enabled", "active", Some(false))), true, false);
    assert!(!lingerless.on && lingerless.sensitive);
    let unknown = background_switch(Some(&read("enabled", "active", None)), true, false);
    assert!(unknown.on, "an unread linger does not turn the switch off");
}

#[test]
fn the_binding_sits_where_the_cli_keeps_it() {
    assert_eq!(
        binding_path(Some(OsStr::new("/srv/config")), Path::new(HOME)),
        Path::new("/srv/config/fermix/service.json")
    );
    let default = Path::new("/home/someone/.config/fermix/service.json");
    assert_eq!(binding_path(None, Path::new(HOME)), default);
    assert_eq!(binding_path(Some(OsStr::new("")), Path::new(HOME)), default);
}

/// A binding file holding `contents`, under a temporary `XDG_CONFIG_HOME`.
fn binding(contents: &str) -> (tempfile::TempDir, PathBuf) {
    let config = tempfile::tempdir().expect("a temporary directory");
    let path = binding_path(Some(config.path().as_os_str()), Path::new(HOME));
    fs::create_dir_all(path.parent().expect("a directory")).expect("the fermix directory");
    fs::write(&path, contents).expect("the binding is written");
    (config, path)
}

fn unreadable(path: &Path) -> String {
    match read_binding(path) {
        Binding::Unreadable(sentence) => sentence,
        other => panic!("{} read as {other:?}", path.display()),
    }
}

#[test]
fn no_binding_file_is_a_fresh_account() {
    let config = tempfile::tempdir().expect("a temporary directory");
    let path = binding_path(Some(config.path().as_os_str()), Path::new(HOME));
    assert_eq!(read_binding(&path), Binding::Absent);
}

#[test]
fn a_binding_that_names_a_usable_home_is_present() {
    let (_config, path) = binding(r#"{"schema_version":1,"home":"/home/someone/.fermix"}"#);
    assert_eq!(read_binding(&path), Binding::Present);
}

#[test]
fn a_binding_the_cli_cannot_decode_is_refused_with_its_own_sentence() {
    let cases = [
        ("{", "could not be read as JSON."),
        (
            r#"{"schema_version":2,"home":"/home/someone/.fermix"}"#,
            "is written in schema version 2, which this Fermix does not read.",
        ),
        (
            r#"{"home":"/home/someone/.fermix"}"#,
            "does not name a home.",
        ),
        (r#"{"schema_version":1}"#, "does not name a home."),
        (r#"["/home/someone/.fermix"]"#, "does not name a home."),
    ];
    for (contents, why) in cases {
        let (_config, path) = binding(contents);
        let expected = format!("The service binding at {} {why}", path.display());
        assert_eq!(unreadable(&path), expected, "{contents}");
    }
}

/// `Binding.validate`: a home the engine would refuse is never taken for a binding.
#[test]
fn a_home_the_engine_would_refuse_is_refused_here_too() {
    let cases = [
        (
            r#"{"schema_version":1,"home":7}"#,
            "The service home must be a path, written as text.".to_owned(),
        ),
        (
            r#"{"schema_version":1,"home":null}"#,
            "The service home must be a path, written as text.".to_owned(),
        ),
        (
            r#"{"schema_version":1,"home":"fermix"}"#,
            r#"The service home must be an absolute path, and "fermix" is not."#.to_owned(),
        ),
        (
            r#"{"schema_version":1,"home":""}"#,
            r#"The service home must be an absolute path, and "" is not."#.to_owned(),
        ),
        (
            r#"{"schema_version":1,"home":"/home/some\none"}"#,
            "The service home contains a control character, which a path cannot carry.".to_owned(),
        ),
        (
            &format!(r#"{{"schema_version":1,"home":"/{}"}}"#, "a".repeat(95)),
            "The daemon.sock path under that home is 108 bytes, over the 107-byte limit this OS \
             allows for a unix socket address. Choose a shorter home."
                .to_owned(),
        ),
    ];
    for (contents, expected) in cases {
        let (_config, path) = binding(contents);
        assert_eq!(unreadable(&path), expected, "{contents}");
    }
}

#[test]
fn a_home_whose_socket_just_fits_is_present() {
    // "/" + 94 bytes + "/daemon.sock" is 107 bytes, the most a Linux socket address holds.
    let contents = format!(r#"{{"schema_version":1,"home":"/{}"}}"#, "a".repeat(94));
    let (_config, path) = binding(&contents);
    assert_eq!(read_binding(&path), Binding::Present);
}

#[test]
fn a_binding_that_cannot_be_opened_names_the_file_and_the_reason() {
    let config = tempfile::tempdir().expect("a temporary directory");
    let path = binding_path(Some(config.path().as_os_str()), Path::new(HOME));
    fs::create_dir_all(&path).expect("a directory where the file belongs");
    let sentence = unreadable(&path);
    let opening = format!(
        "The service binding at {} could not be read (",
        path.display()
    );
    assert!(sentence.starts_with(&opening), "{sentence}");
}

#[test]
fn the_app_runs_the_install_command_people_are_told_to_run_and_reads_its_json() {
    assert_eq!(INSTALL_ARGV[..3].join(" "), INSTALL_COMMAND);
    assert_eq!(INSTALL_ARGV[3], "--json");
}

#[test]
fn the_install_answer_is_its_envelope() {
    let done = r#"{"schema_version":1,"ok":true,"result":{"active":true,"enabled":true}}"#;
    assert_eq!(install_answer(&format!("{done}\n")), Ok(()));
    let refusal = r#"{"schema_version":1,"ok":false,"error":{"code":"activation_timeout","sentence":"The background service was started but did not answer in time."}}"#;
    assert_eq!(
        install_answer(refusal),
        Err("The background service was started but did not answer in time.".into())
    );
}

#[test]
fn an_answer_that_is_not_the_envelope_is_never_taken_for_success() {
    let answers = [
        "",
        "fermix service: installed user-scope unit.\n",
        r#"{"ok":true,"result":{}}"#,
        r#"{"schema_version":2,"ok":true,"result":{}}"#,
        r#"{"schema_version":1,"ok":false}"#,
        r#"{"schema_version":1,"ok":false,"error":{"code":"systemctl_failed"}}"#,
    ];
    for stdout in answers {
        assert_eq!(
            install_answer(stdout),
            Err(INSTALL_UNREADABLE.into()),
            "{stdout}"
        );
    }
}

#[test]
fn the_install_cap_outlasts_the_engines_own_waits_and_is_stated() {
    // `fermix service install` waits up to 90 s for the socket, then up to 90 s for the
    // web address, so its own refusal arrives before this cap stops it.
    assert!(INSTALL_CAP > Duration::from_secs(180), "{INSTALL_CAP:?}");
    let minutes = INSTALL_CAP.as_secs() / 60;
    assert_eq!(
        INSTALL_CAP.as_secs() % 60,
        0,
        "the cap is stated in whole minutes"
    );
    let sentence = install_timed_out();
    assert!(
        sentence.contains(&format!("within {minutes} minutes")),
        "{sentence}"
    );
}

#[test]
fn a_command_that_could_not_run_says_which_and_why() {
    let sentence = install_not_run("Failed to execute child process “fermix” (No such file)");
    assert!(sentence.contains(INSTALL_COMMAND), "{sentence}");
    assert!(sentence.contains("(No such file)"), "{sentence}");
}

#[test]
fn an_install_that_exited_is_judged_by_its_envelope() {
    let ok = br#"{"schema_version":1,"ok":true,"result":{"home":"/home/a/.fermix"}}"#;
    assert_eq!(install_outcome(Ok(Ended::Exited(ok.to_vec()))), Ok(()));
    let refused = br#"{"schema_version":1,"ok":false,"error":{"code":"activation_timeout","sentence":"The service did not answer."}}"#;
    assert_eq!(
        install_outcome(Ok(Ended::Exited(refused.to_vec()))),
        Err("The service did not answer.".into())
    );
    assert_eq!(
        install_outcome(Ok(Ended::Exited(Vec::new()))),
        Err(INSTALL_UNREADABLE.into())
    );
}

#[test]
fn an_answer_that_is_not_text_is_not_understood() {
    let bytes = vec![b'{', 0xff, 0xfe, b'}'];
    assert_eq!(
        install_outcome(Ok(Ended::Exited(bytes))),
        Err(INSTALL_UNREADABLE.into())
    );
}

#[test]
fn an_install_stopped_at_the_cap_says_so() {
    assert_eq!(install_outcome(Ok(Ended::Capped)), Err(install_timed_out()));
}

#[test]
fn an_install_that_could_not_run_says_why() {
    let error = io::Error::new(io::ErrorKind::NotFound, "No such file or directory");
    let sentence = install_outcome(Err(error)).unwrap_err();
    assert_eq!(sentence, install_not_run("No such file or directory"));
}

#[test]
fn a_failed_start_names_the_command_this_account_starts_with() {
    assert_eq!(start_command(true), "systemctl --user start fermix");
    assert_eq!(start_command(false), INSTALL_COMMAND);
}
