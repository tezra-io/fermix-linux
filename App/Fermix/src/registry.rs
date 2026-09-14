//! The surface registry.
//!
//! Every hand-built surface in this door names its counterpart in the macOS
//! door, or carries the sentence that says why it has none. The two-door parity
//! gate walks this list: a surface rendered on macOS and absent here is a
//! recorded exemption rather than a silent gap, and a surface added here
//! without an entry fails `tests/structure.rs`.
//!
//! What is exempt is exactly four things, and all four are properties of the
//! platform rather than decisions of this application: the macOS login item, a
//! permission grant macOS raises and Linux has no equivalent of, a feature the
//! engine does not run on Linux at all, and the picker that feature owns.
//!
//! The Setup assistant's screens are in this list too: they are hand-built, and
//! the macOS door draws every one of them. Two of its screens live inside its
//! assistant's own window view rather than in files of their own, which is why
//! three entries name the same counterpart.

/// One hand-built surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Surface {
    /// The file that draws it, relative to the crate.
    pub file: &'static str,
    /// The macOS counterpart, where there is one.
    pub counterpart: Option<&'static str>,
    /// Why there is none, where there is none.
    pub exemption: Option<&'static str>,
}

/// One thing the macOS door renders that this one does not, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exemption {
    /// What the macOS door calls it.
    pub counterpart: &'static str,
    /// Why this door does not render it.
    pub sentence: &'static str,
}

/// Every hand-built surface this door draws.
pub const SURFACES: &[Surface] = &[
    Surface {
        file: "src/ui/onboarding/welcome.rs",
        counterpart: Some("Onboarding/OnboardingWindowView.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/onboarding/starting.rs",
        counterpart: Some("Onboarding/StartingSurface.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/onboarding/connect_ai.rs",
        counterpart: Some("Onboarding/ConnectAISurface.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/onboarding/about_you.rs",
        counterpart: Some("Onboarding/AboutYouSurface.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/onboarding/applying.rs",
        counterpart: Some("Onboarding/OnboardingWindowView.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/onboarding/ready.rs",
        counterpart: Some("Onboarding/ReadySurface.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/onboarding/boot_failed.rs",
        counterpart: Some("Onboarding/OnboardingWindowView.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/settings/providers.rs",
        counterpart: Some("Settings/Panes/ProvidersPane.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/settings/channels.rs",
        counterpart: Some("Settings/Panes/ChannelsPane.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/settings/integrations.rs",
        counterpart: Some("Settings/Panes/IntegrationsPane.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/settings/meetings.rs",
        counterpart: Some("Settings/Panes/SettingsPaneView.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/settings/computer.rs",
        counterpart: Some("Settings/Panes/ComputerPane.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/settings/permissions.rs",
        counterpart: Some("Settings/Panes/PermissionsPane.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/settings/voice.rs",
        counterpart: Some("Settings/Panes/SettingsPaneView.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/settings/dialogs/secret.rs",
        counterpart: Some("Settings/Rows/SecretRow.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/settings/dialogs/sign_in.rs",
        counterpart: Some("Settings/Panes/ProviderSheets.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/settings/dialogs/model_picker.rs",
        counterpart: Some("Settings/Panes/ProviderSheets.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/settings/dialogs/consent.rs",
        counterpart: Some("Settings/Panes/IntegrationSheets.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/settings/dialogs/oauth_client.rs",
        counterpart: Some("Settings/Panes/IntegrationSheets.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/settings/dialogs/workspace.rs",
        counterpart: Some("Settings/Panes/IntegrationSheets.swift"),
        exemption: None,
    },
    Surface {
        file: "src/ui/settings/dialogs/restart.rs",
        counterpart: Some("Settings/RestartSheet.swift"),
        exemption: None,
    },
];

/// Everything the macOS door renders that this one does not.
pub const EXEMPTIONS: &[Exemption] = &[
    Exemption {
        counterpart: "the login item row",
        sentence: "Opening at login on this platform is a desktop entry in the person's own home \
                   directory rather than a service the operating system registers, so Home writes \
                   that entry and there is nothing here to approve.",
    },
    Exemption {
        counterpart: "computer_use.grant.start",
        sentence: "There is nothing for this application to grant: the helper asks the desktop \
                   for its own session, and on X11 there is no consent to raise at all.",
    },
    Exemption {
        counterpart: "computer history",
        sentence: "The engine runs computer history on macOS only, so this door renders no row \
                   for it anywhere rather than an empty section.",
    },
    Exemption {
        counterpart: "the Choose apps picker",
        sentence: "It belongs to computer history, which this platform does not run.",
    },
];

/// One surface's entry, by the file that draws it.
pub fn surface(file: &str) -> Option<&'static Surface> {
    SURFACES.iter().find(|surface| surface.file == file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_surface_names_a_counterpart_or_an_exemption() {
        for surface in SURFACES {
            assert!(
                surface.counterpart.is_some() != surface.exemption.is_some(),
                "{} names both a counterpart and an exemption, or neither",
                surface.file
            );
        }
    }

    #[test]
    fn every_exemption_says_why() {
        for exemption in EXEMPTIONS {
            assert!(
                exemption.sentence.len() > 40,
                "{} is exempt without a reason a reviewer can weigh",
                exemption.counterpart
            );
        }
    }

    #[test]
    fn the_exemptions_are_the_four_the_design_names() {
        let named: Vec<&str> = EXEMPTIONS
            .iter()
            .map(|exemption| exemption.counterpart)
            .collect();

        assert_eq!(
            named,
            vec![
                "the login item row",
                "computer_use.grant.start",
                "computer history",
                "the Choose apps picker",
            ]
        );
    }

    #[test]
    fn a_file_that_draws_is_found_by_its_own_path() {
        assert!(surface("src/ui/settings/providers.rs").is_some());
        assert!(surface("src/ui/settings/nothing.rs").is_none());
    }
}
