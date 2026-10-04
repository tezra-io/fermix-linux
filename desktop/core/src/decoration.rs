//! The window's buttons, as GTK's `gtk-decoration-layout` writes them: "left:right", each side
//! a comma-separated list of names. The app keeps the desktop's layout, its sides and its order,
//! and adds maximize, which GNOME leaves out unless the user turns it on.

const MAXIMIZE: &str = "maximize";

/// The desktop's `layout` with maximize just after minimize, on whichever side minimize is.
/// Without minimize it goes on close's inner side. A layout that already has maximize, or has
/// neither button, is the user's choice and comes back as written.
pub fn with_maximize(layout: &str) -> String {
    let (left, right) = layout.split_once(':').unwrap_or((layout, ""));
    let (mut left, mut right) = (names(left), names(right));
    if left.iter().chain(&right).any(|name| *name == MAXIMIZE) {
        return layout.to_owned();
    }
    let placed = insert_beside(&mut left, "minimize", Place::After)
        || insert_beside(&mut right, "minimize", Place::After)
        || insert_beside(&mut left, "close", Place::After)
        || insert_beside(&mut right, "close", Place::Before);
    if !placed {
        return layout.to_owned();
    }
    format!("{}:{}", left.join(","), right.join(","))
}

#[derive(Clone, Copy)]
enum Place {
    Before,
    After,
}

fn names(side: &str) -> Vec<&str> {
    side.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect()
}

/// Puts maximize next to `name`, if this side has it.
fn insert_beside(side: &mut Vec<&str>, name: &str, place: Place) -> bool {
    let Some(at) = side.iter().position(|n| *n == name) else {
        return false;
    };
    let at = match place {
        Place::Before => at,
        Place::After => at + 1,
    };
    side.insert(at, MAXIMIZE);
    true
}
