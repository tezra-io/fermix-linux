//! The one motion module.
//!
//! The toolkit owns page and dialog transitions. What the application adds is
//! at most one transition per change, from the two durations below, and zero
//! when the platform asks for reduced motion.

use gtk4 as gtk;
use gtk4::glib;

/// Moving from one surface to another.
pub const NAVIGATION_MS: u32 = 200;
/// Crossing one piece of content over another in place.
pub const CROSSFADE_MS: u32 = 150;

/// The two transitions the application is allowed to add.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    /// Navigation between surfaces.
    Navigation,
    /// A state crossfade in place.
    Crossfade,
}

/// How long a transition runs, in milliseconds.
///
/// Zero when the platform asks for reduced motion, which is a property of the
/// display, so it is read at the moment of the transition rather than cached.
pub fn duration_ms(transition: Transition) -> u32 {
    duration_ms_when(animations_enabled(), transition)
}

/// [`duration_ms`] with the platform's answer supplied, which is what makes the
/// rule testable without a display.
pub fn duration_ms_when(animations_enabled: bool, transition: Transition) -> u32 {
    if !animations_enabled {
        return 0;
    }
    nominal_ms(transition)
}

/// How long a transition runs when animation is allowed.
pub fn nominal_ms(transition: Transition) -> u32 {
    match transition {
        Transition::Navigation => NAVIGATION_MS,
        Transition::Crossfade => CROSSFADE_MS,
    }
}

/// Whether the platform allows animation at all.
///
/// Read from the toolkit's own `gtk-enable-animations`, which is the same
/// setting the toolkit's own transitions honour, so the application and the
/// toolkit can never disagree about it. Without a display there is no settings
/// object and nothing to animate, which reads as reduced motion.
pub fn animations_enabled() -> bool {
    match gtk::Settings::default() {
        Some(settings) => settings.is_gtk_enable_animations(),
        None => false,
    }
}

/// Keep one stack's transition on the platform's own answer, which a person can
/// change while the window is open.
///
/// Read once and the window goes on animating after reduced motion is switched
/// on, which is the one setting the redlines say the application never
/// overrides.
pub fn follow(stack: &gtk::Stack, transition: Transition) {
    let apply = move |stack: &gtk::Stack| stack.set_transition_duration(duration_ms(transition));
    apply(stack);

    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    settings.connect_gtk_enable_animations_notify(glib::clone!(
        #[weak]
        stack,
        move |_| apply(&stack)
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_durations_are_the_declared_ones() {
        assert_eq!(nominal_ms(Transition::Navigation), 200);
        assert_eq!(nominal_ms(Transition::Crossfade), 150);
    }

    #[test]
    fn reduced_motion_is_zero_for_every_transition() {
        assert_eq!(duration_ms_when(false, Transition::Navigation), 0);
        assert_eq!(duration_ms_when(false, Transition::Crossfade), 0);
    }

    #[test]
    fn allowed_motion_is_the_nominal_duration() {
        assert_eq!(
            duration_ms_when(true, Transition::Navigation),
            NAVIGATION_MS
        );
        assert_eq!(duration_ms_when(true, Transition::Crossfade), CROSSFADE_MS);
    }
}
