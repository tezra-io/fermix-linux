//! The permission ledger (M38 §7.4) and the platform statements the Voice,
//! Meetings and Computer panes show (M38 §6.5, §8.6). One home for each
//! string, so Permissions and the capability panes cannot disagree.

pub struct Right {
    pub title: &'static str,
    /// Who holds the right.
    pub principal: &'static str,
    /// How a person takes it back.
    pub revoke: &'static str,
    /// Where the durable record of it lives.
    pub artifact: &'static str,
}

pub const RIGHTS: [Right; 7] = [
    Right {
        title: "Microphone and voice",
        principal: "Nobody. PipeWire serves whoever asks.",
        revoke: "There is nothing to take back. Quit the program, or mute the microphone in \
                 PipeWire or in your sound settings.",
        artifact: "Nowhere. Nothing records that access happened.",
    },
    Right {
        title: "Screen capture",
        principal: "A screen cast session tied to the desktop portal identity this program \
                    presents.",
        revoke: "Your desktop\u{2019}s sharing settings.",
        artifact: "A restore token the helper writes and replaces on every session.",
    },
    Right {
        title: "Keyboard and pointer control",
        principal: "A remote desktop session with input devices, taken together with any capture \
                    sources it carries.",
        revoke: "Your desktop\u{2019}s sharing settings.",
        artifact: "One restore token for the whole session, held by the helper.",
    },
    Right {
        title: "Background service",
        principal: "Your own user service manager, plus permission to keep it running after you \
                    log out.",
        revoke: "Turn the background service off on the Home page.",
        artifact: "The packaged service file, your own enablement of it, and the home it is \
                   bound to.",
    },
    Right {
        title: "Open at login",
        principal: "A desktop entry in your own home directory.",
        revoke: "Turn off opening at login on the Home page, or remove the entry yourself.",
        artifact: "The entry itself, in your own home directory.",
    },
    Right {
        title: "Administrator actions",
        principal: "Your system\u{2019}s authorization service, asked only when you run one of \
                    the commands Fermix prints.",
        revoke: "Nothing is kept, so there is nothing to take back. A granted answer is commonly \
                 remembered for a few minutes.",
        artifact: "Nowhere. Fermix installs no authorization rule of its own.",
    },
    Right {
        title: "Browser automation",
        principal: "On Ubuntu, the confinement profile that ships for the browser at its packaged \
                    path.",
        revoke: "Replace or remove that profile, which is a decision for the whole machine.",
        artifact: "A profile on the host, keyed to the browser\u{2019}s path. A browser \
                   installed elsewhere is not covered by it.",
    },
];

pub const PLATFORM_FACT: &str = "On this platform a permission is something Fermix asserts about \
    itself and keeps for itself, not something the operating system verifies and stores. Lose \
    what Fermix keeps and the consent is gone; copy it and the consent moves with it.";

/// M38 §6.5's microphone statement opens with this. Where the statement folds
/// away, this heads it; where it is shown whole, `microphone_statement` joins them.
pub const MICROPHONE_HEADLINE: &str = "Linux has no microphone permission";

pub const MICROPHONE_DETAIL: &str = "Nothing asked you, nothing appears in your system settings, \
    and there is nothing to revoke. While Fermix is running it can open the microphone at any \
    time, and so can any other program you run. Your real controls are to not run it, to mute \
    the microphone in your sound settings or in PipeWire, or to run it in a sandbox that \
    withholds audio, which also stops it playing sound. On macOS the operating system asks \
    first. On Linux it does not.";

/// The whole statement, word for word.
pub fn microphone_statement() -> String {
    format!("{MICROPHONE_HEADLINE}. {MICROPHONE_DETAIL}")
}

pub const MEETINGS_SLEEP_STATEMENT: &str = "Fermix cannot keep this computer awake during a \
    meeting. If the machine suspends, the recording stops. Adjust your power settings before a \
    long meeting.";
