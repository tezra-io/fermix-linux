# Fermix for Linux, design redlines

**Status:** binding for v1
**Updated:** 2026-09-13
**Parent:** `fermix/docs/design/MILESTONE_38_LINUX_COMPANION_APP.md` (M38), whose §5, §6 and §6.8 this
document turns into the rules an implementer and a reviewer both hold. Where M38 and this file disagree,
this file is the narrower reading and wins for the Linux application; M38 wins for the engine, the
packages and the release order.

The macOS application (`fermix-macos`) is the sibling, not the template. Its interaction decisions port
(one window, Settings inside it, native progress, one Restart action, the daemon owns every sentence).
Its pixels do not: no palette, no glass, no drawn cards, no fixed windows.

## 0. The one sentence

**Consume the platform, decide nothing the daemon already decided, and make every task reachable from
the keyboard in fewer steps than a web page would need.**

Everything below is that sentence applied to a surface.

## 1. What good looks like

A GNOME user opens Fermix and it reads as a GNOME application built by someone who cared: an
`AdwApplicationWindow`, a flat header bar, a labelled sidebar, boxed lists, the system's accent on the one
switch that is on, the system's font, nothing painted. A Plasma user sees the same application and it is
legible, complete and quick; it is not a Breeze application and does not pretend to be.

Calm. Status is a word in a row, never a hero, a dot, a halo or a pulse. An empty state is one sentence.
A failure is three: what happened, what is untouched, the one next action. Nothing animates to say
"working" except a spinner on the row that is working.

Fast. Ready to draw within a second of launch. Every list is keyboard-traversable; every dialog has one
default action on Enter and closes on Escape; every settings pane is reachable by typing its name.

Honest. A row that cannot be acted on says why in the row. A capability Linux does not gate says so in
the words the copy catalogue owns. Nothing prompts on render, nothing infers a state the daemon did not
publish, nothing degrades silently.

## 2. Structure

| Element | Rule |
|---|---|
| Window | Exactly one `AdwApplicationWindow`. Default 880×560 logical px, minimum 460×440, resizable, geometry restored from `$XDG_STATE_HOME/fermix-desktop/state.json`. Nothing paints a ground; `--window-bg-color` is the ground |
| Toolbar | `AdwToolbarView` with one `AdwHeaderBar`. Title is the current page title. Trailing: at most one prominent (`suggested-action`) button while its condition holds, then the primary menu under `open-menu-symbolic`. Never more than three trailing children |
| Navigation | One `AdwNavigationSplitView` per window (a gate counts them). Sidebar 216 px: a `GtkListBox` in navigation-sidebar style with Home, Doctor, Logs, and one pinned Settings row at the bottom in the same list and the same focus order. An `AdwBreakpoint` collapses the split view below the width budget of §5 |
| Settings | A presentation of the same window: the sidebar becomes the searchable pane list in four groups, the detail becomes the pane. Entered by the pinned row, the primary menu, or `Ctrl+comma`. Left by the Back control (`go-previous-symbolic`, accessible name "Back to Fermix") or Escape. Selected pane and scroll position survive leaving and returning; only the selected pane persists across launches |
| Setup assistant | An `AdwNavigationView` inside the same window, sidebar hidden. Stable bottom bar: leading Back (or Cancel on Starting), trailing exactly one default action. Content scrolls above the bar; the bar never moves |
| Content column | Home, Settings panes, Doctor, Recovery and every assistant screen sit in an `AdwClamp` with `maximum-size` 660 and `tightening-threshold` 480. Logs uses the full detail width |
| Containers | The app draws no container. `AdwPreferencesGroup`, boxed lists and `AdwStatusPage` draw every box, hairline and separator. A `GtkFrame`, a drawn border, a custom separator or a CSS background on a content widget is a defect |
| Dialogs | `AdwDialog` for credential entry, pickers, plugin consent and the OAuth client; `AdwAlertDialog` for Restart, Finish Updating and destructive confirmations. One default response, Cancel always present, Escape cancels, secrets cleared on close. Sized by content, never fixed |
| Empty, error, recovery | `AdwStatusPage` with a symbolic icon, a title and one sentence, plus at most one button |
| Tray | None in v1. No `StatusNotifierItem`, no tray copy anywhere in the product |
| Menus | The primary menu holds: Settings…, Run Doctor, Restart Fermix…, Keyboard Shortcuts, About Fermix, Quit. No menu bar, no global menu path |

## 3. Keyboard and clicks

The mandate is fewer steps than the browser setup page and full keyboard reach. These are gates.

| Task | Path | Steps |
|---|---|---|
| Open Settings | `Ctrl+comma`, or the pinned row | 1 |
| Find a pane | While Settings is shown, start typing: the pane list's `GtkSearchEntry` is the list's key-capture widget, so typing anywhere in the sidebar filters it. `Enter` opens the first match | type + Enter |
| Switch a feature | Focus the switch row, `Space`. The write is sent on toggle; a refusal reverts it and shows the sentence under the row | 1 |
| Add an API key | Providers → row's inline "Add Key…" → dialog → paste → `Enter` | 3 |
| Sign in | Providers → row's inline "Sign In" → browser opens; the dialog shows the URL as copyable text if it did not | 2 |
| Enable a plugin that is not installed | Integrations → the row's switch. One gesture: consent dialog (`Enter` accepts), install job, enable, then the detail opens only if the daemon's next step needs the person | 2 |
| Run Doctor | `Ctrl+R` on Doctor, or the primary menu | 1 |
| Restart | `Ctrl+Shift+R`, or the Settings toolbar action, or the Home attention row | 1 + confirm |
| Jump between pages | `Ctrl+1` Home, `Ctrl+2` Doctor, `Ctrl+3` Logs; `Alt+Left` and `Escape` go back | 1 |
| Search the current surface | `Ctrl+F` focuses the search entry of Settings, Integrations or Logs | 1 |
| Shortcuts reference | `Ctrl+question` | 1 |
| Quit | `Ctrl+Q`; `Ctrl+W` closes the window, which quits (the daemon is not the window's child) | 1 |

Rules behind the table:

- Every action is a `GAction` on the application or the window, registered once in one action map,
  with its accelerator declared beside it. Menus, the header bar, the shortcuts dialog and the
  notification actions bind to that map. Nothing is reachable from only one place.
- Every interactive widget is focusable, has an accessible role and a name, and shows the toolkit focus
  ring. The app ships no CSS that touches `outline`.
- `Enter` in a single-line entry commits it; `Escape` restores the daemon's value and sends no write.
  Focus loss commits a changed value. A refresh never rebuilds the focused control or moves focus.
- A row that opens something says so with a trailing `go-next-symbolic` and is activatable; its switch
  is a separate focus stop and does not activate the row.
- No task needs a mouse hover to discover its action. Hover feedback is the toolkit's and reveals nothing.

## 4. Colour, type, motion, icons

- **No colour literal in the application sources or its CSS.** Everything is a libadwaita variable:
  `--window-bg-color`, `--view-bg-color`, `--card-bg-color`, `--accent-bg-color`/`--accent-fg-color` on
  fills, `--accent-color` for accent text, the `--success-*`, `--warning-*`, `--error-*` triples, and
  `--dim-opacity` for dimmed text. Brand blue exists on the icon and the wordmark only. A build gate greps
  for `#[0-9a-fA-F]{3,8}` and `rgb(`/`rgba(` outside `resources/icons/` and `resources/brand/`.
- **Type** is the system font through style classes only: `title-1` for the page heading, `heading` for a
  group, default body for controls, `caption` for supporting text, `monospace` for identifiers and log
  lines. No `font-size`, no `font-family`, no `font-weight` in CSS. Readable captions are body-sized
  captions; the `dim-label` class is used only where the toolkit itself keeps 4.5:1.
- **Spacing** comes from one `metrics` module: 6, 12, 18, 24, 36, 48. Outer gutters 24 with the sidebar
  shown, 18 collapsed; 24 between groups; 12 between a heading and its content; app-arranged rows carry
  12 vertical padding; the leading artwork slot is 36 wide. Toolkit padding inside rows is never
  overridden.
- **Status is never colour-only.** Doctor uses text pills (`P`, `W`, `F`, `U`, `S`, `C`, `T`, `N`) with
  the word beside them; Home's Status is a word; a plugin's state is its status sentence.
- **Motion** is the toolkit's page and dialog transitions. The app adds at most one transition per
  change, from one `motion` module: 200 ms for navigation, 150 ms for a state crossfade, and 0 when
  `gtk-enable-animations` is false. Polls never replay entrances; spinners run only for pending work.
- **Icons** are Adwaita symbolic icons from the platform theme. The application icon is authored to the
  GNOME icon guidelines (128 px canvas, flat colour, a symbolic pair), named `io.tezra.Fermix`, and is not
  the macOS master exported. The mascot and wordmark appear on Welcome, Ready and About only.
- **Vendor marks** ship byte-for-byte from `resources/VendorMarks/` with `PROVENANCE.json` and
  `ROSTER.json` beside them (the same records the macOS app keeps; the bytes are the vendors' own). A mark
  draws at the toolkit's row size inside the 36 px slot, never recoloured, never inset when its file
  carries its own ground, and with its recorded accessibility label. A vendor without a retrievable mark
  is its text name beside a neutral symbolic icon. Nothing invents a monogram.

## 5. Layout budgets

| Measurement | Value |
|---|---|
| Two-column width budget | sidebar 216 + detail 420 + gutters 2×24 = 684 logical px at text scale 1.0, expressed as `sp` so larger text collapses sooner |
| Content clamp | 660 max, 480 tightening |
| Row minimum height | 48, or the toolkit's larger natural minimum; text growth is never clipped |
| Leading artwork slot | 36 wide, shared by every list that draws a mark |
| Window | 880×560 default, 460×440 minimum |
| Logs header floor | 509. Logs holds the window wider than every other surface, and it is allowed to: its header carries a search entry, a level filter, Pause and Export, each already shrinking as far as it will go, and section 3 forbids hiding the search behind a gesture that has to be discovered. The header spans the window rather than the detail, so this is the window's minimum while Logs is showing; navigation still collapses at the 684 budget above, well below it. `LOGS_HEADER_FLOOR` in `tests/widgets.rs` is the ratchet, and it fails if the number grows |

No persistent surface scrolls horizontally. Logs may scroll long lines.

## 6. Surfaces, in the order a person meets them

### Home
Page title "Home". Group **Background**: row `Status` with the state word (`Running`, `Setup required`,
`Restart to finish updating`, `Fermix isn't running`); `AdwSwitchRow` "Run in the background" (service
enablement through the CLI); `AdwSwitchRow` "Open at login" (the XDG autostart entry). Group
**Attention**: one `AdwActionRow` per daemon-reported gap with exactly one trailing button; empty is one
centred sentence "Nothing needs your attention". Group **Runtime details**: an `AdwExpanderRow`, collapsed
by default, holding plain labelled rows: Engine, Management protocol, Uptime, Provider, Channels, Skills,
Tools, Service, Session. Toolbar action while it applies: "Continue Setup" or "Finish Updating".

### Setup assistant
Welcome → Starting → Connect Your AI → About You → Applying → Ready, with Boot Failed and Recovery as
replacements. Starting and Applying are a checklist of `AdwActionRow`s whose prefix is one of: a static
pending glyph, an `AdwSpinner`, `emblem-ok-symbolic`, or `dialog-error-symbolic`; there is no mascot on
these two screens. The finish gate is the daemon's. Every activation failure lands on a named state
with its own sentence within 90 seconds.

### Settings
Four groups, thirteen panes, named exactly: Assistant (Providers, Personality, Memory), Connections
(Channels, Integrations), Capabilities (Voice, Meetings, Computer, Coding agents, Search, Images),
System (Sandbox, Permissions). Descriptor panes render through one `DescriptorForm`; the six hand-built
panes follow M38 §5.7. One Restart… action in the header bar while a restart is pending. The
external-change banner (`AdwBanner`) has one action, "Reload Settings From Disk"; an unreadable file
routes to Recovery and offers no reload.

### Doctor
Summary line ("Healthy" or "N failed", "Checked just now"), then one `AdwPreferencesGroup` of rows: pill,
name, summary, and for a failed row the remediation title with its one button; evidence in an expander
beneath. Toolbar: "Run Network Checks" (labelled with its cost) and a menu with "Export Support Bundle"
and "Show Log Folder".

### Logs
Edge to edge `GtkListView` of monospace rows: local time to the millisecond, level as a word, message.
Toolbar: search entry, level dropdown, Pause toggle, Export. Two-second poll while visible and unpaused.
One always-visible caption line naming the file this surface reads and the `journalctl --user -u fermix`
command for stream output.

### Recovery
`AdwStatusPage`: the typed cause as the title, one sentence, an expander with the offline evidence, and
buttons Retry, Export Diagnostics, Show Journal Command.

## 7. Copy

One catalogue, `src/copy.rs`, one entry per key: the English sentence-case source and its casing class.
`header` rows (window and page titles, group headings, buttons, menu items, dialog titles) render in
Header Capitalization; `sentence` rows (body, field labels, switch labels, captions, refusals) render as
written. Ellipsis is U+2026 and appears only on a label whose action needs further input.

Forbidden anywhere in the catalogue: em dashes, exclamation marks, "please wait", placeholder words,
`mix `, `config.toml`, an environment variable name, and the words `Save`, `Apply`, `Submit` as control
labels. A `sentence` row that is Header Capitalized fails; a `header` row that is not fails; a `header`
row ending in a period fails.

Every status word, refusal sentence, remediation title, restart reason and plugin sentence comes off the
wire. For `invalid_params` and `config_unreadable` the app renders `details.sentence` when present and
`message` otherwise. A literal status or refusal sentence in the Rust sources is a build failure.

The Linux-only moments (linger denied, loginctl absent, secret store states, microphone honesty,
version skew, system-scope refusal) carry M38 §6.5's text verbatim in the catalogue. Two Linux-only
sentences are defined here because M38 leaves them to the implementation:

- Voice pane: "Voice is configured here and used from a companion application. The companion exists for
  macOS today, and there is no Linux companion yet."
- Meetings pane: "Fermix cannot keep this computer awake during a meeting. If the machine suspends, the
  recording stops. Adjust your power settings before a long meeting."

## 8. Accessibility gates

- Every interactive node has a role and a name in the AT-SPI tree (`GTK_A11Y=test` dump in CI).
- Full keyboard path on every surface; no focus trap; the focus ring is never suppressed.
- Contrast: the app introduces no colour, so contrast is the toolkit's; captions use body-sized text.
- High contrast, reduced motion and text scaling are honoured because nothing overrides them; snapshots
  run at text scale 1.0 and 1.25.
- Every shipped mark resolves to its recorded accessibility label.

## 9. Evidence

`docs/design/captures/` holds reference PNGs rendered by the app's own capture mode against the fixture
daemon, one per reference state in §6.8 of M38, light and dark. Each capture is listed in
`docs/design/captures/INDEX.md` with fixture id, commit, GTK and libadwaita versions, window size, text
scale, colour scheme and the reviewer's decision. A capture that exists is not an accepted capture.
