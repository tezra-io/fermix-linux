//! The window's buttons: the desktop's own layout, with maximize beside minimize.

use fermix_client::decoration::with_maximize;

#[test]
fn maximize_sits_beside_minimize_on_either_side() {
    assert_eq!(
        with_maximize("appmenu:minimize,close"),
        "appmenu:minimize,maximize,close"
    );
    assert_eq!(
        with_maximize("close,minimize:appmenu"),
        "close,minimize,maximize:appmenu"
    );
}

/// GNOME's default has close alone: maximize goes on its inner side.
#[test]
fn without_minimize_maximize_sits_inside_close() {
    assert_eq!(with_maximize("appmenu:close"), "appmenu:maximize,close");
    assert_eq!(with_maximize("close:appmenu"), "close,maximize:appmenu");
}

#[test]
fn a_layout_with_maximize_or_without_window_buttons_is_kept_as_written() {
    for layout in [
        "appmenu:minimize,maximize,close",
        "maximize:close",
        "icon:",
        ":",
        "",
    ] {
        assert_eq!(with_maximize(layout), layout);
    }
}

#[test]
fn spaces_and_empty_names_in_the_desktops_layout_are_tolerated() {
    assert_eq!(
        with_maximize("appmenu: minimize , close"),
        "appmenu:minimize,maximize,close"
    );
    assert_eq!(with_maximize(",close,:"), "close,maximize:");
}

/// GTK reads a layout without a colon as all on the left.
#[test]
fn a_layout_without_a_colon_is_all_on_the_left() {
    assert_eq!(with_maximize("close,minimize"), "close,minimize,maximize:");
}
