# Fermix for Linux, final design: setup-first shell, chat one click away

> **Status:** the design record written before the app was built (2026-09-24), kept because the
> code cites its sections. Where the shipped app differs, the code is right. The main differences:
> the sidebar is Chat, Voice, Home, Doctor and Logs, with Settings pinned below them and all of
> Settings built; a provider row leads with one button and keeps the rest behind its ⋮ menu instead
> of a split button; Chat renders Markdown, tool calls and pictures. The open questions at the end
> were settled in the build.

Base: design B (setup-first control center). Grafted from A: the not-ready Chat page that routes to the
fix, the "Start chatting" toast action, inline browser-fallback instead of a dialog, the `AdwBanner`
daemon-down state that keeps a transcript, per-screen request-refused states, the 1 s job poll, and the
Chat rules for scroll, Stop, reconnect and errors. Every widget below was checked against the GNOME 50
runtime typelib (`Adw-1.typelib`, `Gtk-4.0.typelib`: GTK 4.22.4, libadwaita 1.9) and against
libadwaita-rs 0.9.2, which binds through `v1_9`. Versions in brackets are first appearance. Every
sentence about daemon state is the daemon's; the app composes only labels, verbs and the short
explanations quoted here. Copy is sentence case, no wire tokens on screen.

## 1. Navigation and information architecture

```
AdwApplicationWindow (default 880×560, min 460×440)
└─ AdwToastOverlay                                    every acknowledgement lands here
   └─ AdwNavigationSplitView [1.4]  (AdwBreakpoint [1.4]: collapse below 640sp; sidebar 216)
      ├─ sidebar: AdwNavigationPage "Fermix"
      │  └─ AdwToolbarView [1.4] + AdwHeaderBar (title only)
      │     └─ GtkListBox .navigation-sidebar, one GtkListBoxRow (icon + label) per page:
      │        Home · Providers · Chat        (slice 1: Home, Providers; slice 2 adds Chat)
      └─ content: AdwNavigationPage (title follows the selected row)
         └─ AdwToolbarView + AdwHeaderBar
            │  trailing: [one contextual .suggested-action button] [primary menu: Keyboard
            │  shortcuts, About Fermix, Quit]. Never more than two trailing children.
            └─ AdwViewStack: home · providers · chat   (each child an AdwClamp, max 660; Chat 720)
```

- This is the shell the owner already liked (sidebar + preference pages). One window, one split view,
  one stack. Ctrl+1/2/3 select sidebar rows; Alt+Left does nothing (no page stack in slice 1).
- **Later slices slot in as sidebar rows, nothing moves:** a "Settings" section header followed by one
  row per `settings.sections` pane, each a descriptor-driven `AdwPreferencesPage` in the same stack;
  **Voice** a row under Chat (and a microphone `GtkToggleButton` in the composer). A **first-run
  assistant**, if ever built, is an `AdwDialog` [1.5] over Home. Providers stays a top-level row for
  good: it is the one setting a companion cannot work without, so sign-in never grows past two clicks.
- Window opening rule: connect `daemon.sock`, `hello`, `setup.state.get`. Land on **Home** in slice 1.
  In slice 2 land on **Chat** when readiness is `ready`, otherwise Home; the last page is not remembered.
- Connection is one object owned by the window and passed to each page. Each page renders from
  `(connection state, last setup.state.get, in-flight jobs, last_event)`; `setup.state.get` is re-read
  after every write or terminal job, then every 5 s while a job runs and every 30 s idle.

## 2. Slice-1 screens as widget trees

### Home (page title "Home")
```
AdwPreferencesPage
├─ AdwPreferencesGroup "Fermix"
│  ├─ AdwActionRow "Status"        suffix GtkLabel .dim-label: Ready | Setup required |
│  │                                Restart to finish updating | Not running | Update needed
│  ├─ AdwActionRow "Answers with"  suffix GtkLabel: "<primary label> · <default_model>" or
│  │                                "No provider yet"; activatable, go-next-symbolic → Providers
│  └─ AdwActionRow "Version"       suffix GtkLabel .dim-label: engine version from hello
├─ AdwPreferencesGroup "Attention"
│  │  one AdwActionRow per readiness failure (gating first) and per restart reason:
│  │  title = daemon sentence, subtitle = daemon detail, exactly one suffix GtkButton:
│  │  pane providers → "Open providers"; restart → "Restart Fermix…"; a pane this app has not
│  │  built → no button, subtitle "Not available in this version yet"
│  └─ empty: centred GtkLabel .dim-label "Nothing needs your attention"
└─ AdwPreferencesGroup
   └─ AdwExpanderRow "Details" (collapsed): Engine, Protocol, PID, Providers signed in (n), Channels (n)
```
Header contextual button: "Restart Fermix…" while `restart.required` (`AdwAlertDialog` [1.5] "Restart
Fermix?", body = the daemon's reasons joined by newlines, Cancel / Restart suggested). Restart =
`lifecycle.prepare` → `lifecycle.commit`; `fermix.service` has `Restart=always`, so systemd brings it
back and the app runs the reconnect loop of §5. If `prepare` answers `busy`, the dialog shows the
daemon's sentence and keeps its buttons.

### Providers (page title "Providers")
```
AdwPreferencesPage
└─ AdwPreferencesGroup "Providers"
   description "Fermix answers with the primary provider. Sign in to one to start."
   └─ one AdwActionRow per setup.state.get provider row, daemon order
      prefix   GtkImage 32 px vendor mark from resources, else AdwAvatar with the initial
      title    the daemon's label ("OpenAI Codex (ChatGPT)", "Anthropic", …)
      subtitle the state sentence (§3); failures in .error
      suffix   GtkStack (crossfade 150 ms), exactly one child visible:
               · AdwSplitButton (not configured): main verb = fewest-typing way in, menu = the rest
               · GtkButton "Add key…" (api_key-only providers: a split button with no menu is a button)
               · AdwSpinner [1.6] + GtkButton "Copy link" (browser jobs only) + GtkButton "Cancel"
               · GtkMenuButton view-more-symbolic (configured): Make primary · <other ways in> ·
                 Sign out / Remove key. Subtitle gains " · Primary" on the primary row.
               · nothing (ollama: subtitle "Nothing to enter"; its menu holds only Make primary)
```
No detail page and no primary combo row in slice 1: every action is on its row, and "Primary" is read
from the subtitle and from Home. Dialogs (`AdwDialog` [1.5], content-sized, Escape cancels, the entry
is cleared on close):
- **Key dialog**: title "<Label> API key", `AdwPreferencesGroup` with `AdwPasswordEntryRow` [1.2] "API
  key", description "Stored in your keyring. Fermix never shows it again." Buttons Cancel / Add
  (suggested, default, insensitive while empty). The same dialog titled "Anthropic setup token" with
  description "Run claude setup-token in a terminal and paste what it prints."
- **Sign-out confirm**: `AdwAlertDialog` (§3 D).
- `AdwShortcutsDialog` [1.8] for Ctrl+question, `AdwAboutDialog` [1.5]. Both verified present.
- Browser launch: `gtk::UriLauncher` [GTK 4.10], which goes through the OpenURI portal inside the
  Flatpak and needs no extra permission. No fallback dialog: the row carries the fallback (§3 A).

Present and deliberately unused: `AdwSidebar`/`AdwSidebarItem` [1.9] (new; `GtkListBox
.navigation-sidebar` is the proven shape), `AdwInlineViewSwitcher`/`AdwToggleGroup`/`AdwWrapBox` [1.7],
`AdwBottomSheet` [1.6], `AdwMultiLayoutView` [1.6]. Nothing in this spec is flagged unsure.

## 3. Provider credential flows, one state table per method

Shared rules. `job.get` is polled every 1 s (Step 0 showed `verifying` is too brief to see at 2 s),
bounded at the job's budget plus 5 s, after which the row renders the daemon's terminal view or, if
the daemon is unreachable, the §5 state. A job carries `kind` and `name`, so a running `auth` or
`auth_import` job found by `job.list` on window open is adopted into its provider row. Every terminal
state re-reads `setup.state.get` before it is drawn. Failure sentences are the daemon's
`failure.sentence`, rendered in the subtitle with `.error`; the app never composes one. The `busy`
error on any start call renders "A sign-in is already running" with Cancel, and the row adopts the
job via `job.list`. The suffix crossfades between states so the eye sees every transition.

**A. Browser sign-in** (`auth.start`; openai_codex, xai)

| State | Subtitle | Suffix | User clicks |
|---|---|---|---|
| Not signed in | "Not signed in" | AdwSplitButton **Sign in** ▾ (ChatGPT menu: Import from Codex CLI; xAI menu: Use an API key…) | Sign in → `auth.start`, then `UriLauncher` on `authorize_url` |
| Starting (phase binding) | "Starting sign-in" | AdwSpinner, Cancel | — |
| Awaiting browser | "Finish signing in in your browser. Fermix never sees your password." | AdwSpinner, **Copy link**, Cancel | finishes in the browser |
| Browser did not open (UriLauncher error) | "Your browser did not open. Copy the link and open it yourself." | **Copy link**, Cancel | Copy link |
| Verifying | "Checking the sign-in" | AdwSpinner | — |
| Completed | "Signed in just now" (→ "Signed in" after 2 min, §4) | menu button | — ; toast "Signed in to ChatGPT" |
| Failed / timed out | the daemon's `failure.sentence` | **Sign in** ▾ | Sign in |
| Cancelled | "Sign-in cancelled" | **Sign in** ▾ | — |
| Expired later (`token_state: expired`) | "Sign-in expired" | **Sign in again** ▾ | Sign in again → `auth.start` |

The URL lives in memory for the job's life only and Copy link goes insensitive once `expires_in_ms`
has elapsed. **Clicks, fresh window → browser open, ChatGPT: 2** (sidebar "Providers", **Sign in**).
From Home in setup-required state also 2 ("Open providers", **Sign in**).

**B. Import an existing login** (`auth.import.start claude_code` / `codex_cli`)

| State | Subtitle | Suffix | User clicks |
|---|---|---|---|
| Not signed in (Anthropic) | "Not signed in" | AdwSplitButton **Import from Claude Code** ▾ (Paste a setup token… / Use an API key…) | the main verb |
| Reading (phase reading_keychain) | "Reading your Claude Code login" | AdwSpinner, Cancel | — |
| Verifying | "Checking the login" | AdwSpinner | — |
| Completed | "Signed in just now with your Claude Code login" | menu button | — ; toast "Signed in to Anthropic" |
| Failed | daemon sentence (e.g. no login found) | **Import from Claude Code** ▾ | picks another way from ▾ |
| ChatGPT variant | menu item "Import from Codex CLI" on the ChatGPT split button; same states | | |

Anthropic never shows a browser sign-in: the daemon refuses `auth.start anthropic`. **Clicks, fresh
window → signed in, Anthropic: 2** (sidebar "Providers", **Import from Claude Code**; the job finishes
hands-free). Via setup token: 4 + one paste (Providers, ▾, "Paste a setup token…", paste, Add).

**C. Paste a token or API key** (`secret.set <provider>_api_key` / `anthropic_setup_token`)

| State | Subtitle | Suffix / where | User clicks |
|---|---|---|---|
| No key (openai, openrouter, mistral, venice) | "No API key" | GtkButton **Add key…** | Add key… → key dialog |
| Dialog open | | key dialog | paste, **Add** or Enter |
| Saving | "Adding the key" | AdwSpinner | — |
| Saved | "Key added just now" → "Key added" | menu button (Make primary / Replace key… / Remove key) | — ; toast "Key added for OpenAI" |
| Refused (`secret_store_failed`, `invalid_params`) | | dialog stays open, the daemon's sentence under the entry in `.error`, Add re-enabled | fix and retry, or Cancel |

**D. Sign out** (`auth.logout` for sign-ins, `secret.clear` for keys; one verb each in words)

| State | Row | User clicks |
|---|---|---|
| Configured | "Signed in" / "Key added" · menu button | menu → **Sign out** / **Remove key** |
| Confirm | `AdwAlertDialog` "Sign out of Anthropic?" body "Fermix forgets this sign-in on this computer. Nothing is revoked at the provider." Cancel / Sign out (destructive) | **Sign out** |
| Working | "Signing out" · AdwSpinner | — |
| Done | "Not signed in" · split button; toast "Signed out of Anthropic". If it was primary: Home "Answers with" → "No provider yet", Attention gains the daemon's provider failure, Chat shows its not-ready page | — |
| Refused | the daemon's sentence in the subtitle `.error`; row otherwise unchanged; never a modal | — |

**E. Make primary** (`providers.set_primary`)

| State | Row | User clicks |
|---|---|---|
| Configured, not primary | menu → **Make primary** (Providers, ⋯, Make primary: 3 clicks) | the item |
| Working | menu button insensitive, subtitle "Switching" | — |
| Done | subtitle gains " · Primary", the old primary loses it; Home "Answers with" changes; toast "Fermix now answers with Anthropic"; if the daemon reports `restart.required`, the header gains **Restart Fermix…** and Home's Attention gets the daemon's reason row | — |
| Not configured | item absent (only configured rows have the menu) | — |
| Refused | the daemon's sentence in the subtitle `.error` | — |

Token state on the wire maps to copy: valid → "Signed in", expired → "Sign-in expired" (main verb
becomes the method in `auth_mode`), missing → "Not signed in"; `present_key` → "Key added".

## 4. Acknowledging completion when the row does not change

Step 0 showed a ChatGPT re-sign-in leaves the wire row byte-identical, so acknowledgement never
depends on a diff. The Providers page keeps `last_event: {provider, verb, at}` in memory for the
window's life, set when a job the app started or adopted reaches `completed` or a write lands.

1. **The row says so.** Subtitle "Signed in just now" for two minutes (one bounded `glib::timeout`),
   then relaxes to the plain wire sentence "Signed in". A re-sign-in always produces a new sentence.
2. **The suffix moves.** The `GtkStack` crossfades spinner → menu button, so the end is visible even
   without reading.
3. **A toast.** "Signed in to ChatGPT" (5 s) from the `AdwToastOverlay`, even if the window is on
   another page. When readiness just flipped to `ready` the toast carries an action: "Start chatting"
   (switches to Chat) in slice 2, "Open Home" in slice 1.
4. **Home moves.** Status changes word, "Answers with" fills in, the providers Attention row leaves.
5. **No invented names.** `account_label` is null for ChatGPT; "Signed in" is the whole claim.

Failures use channels 1, 2 and 4 with the daemon's sentence and no toast: the row carries it.

## 5. Daemon not running and daemon refused

Detection: connecting to `~/.fermix/daemon.sock` fails with ENOENT (missing) or ECONNREFUSED (stale
socket); `hello` answers `client_too_old` / `daemon_too_old`; a request answers an error object. The
window keeps one `ConnectionState` and every page draws from it. A connection that drops mid-session
flips every page on the next poll; in-flight rows show "Lost contact with Fermix" and stop polling.

| Screen | Socket missing | Connection refused | Version refused | Request refused |
|---|---|---|---|---|
| Home | Status "Not running"; Attention holds one row "Fermix is not running" subtitle "Sign-in and chat need it" button **Start Fermix**; Details collapsed and empty | Status "Not responding"; same row, subtitle "Its socket is there but nothing answers", button **Restart Fermix** | Status "Update needed"; Attention row with the daemon's sentence, button **Update Fermix** (daemon_too_old) or **Update the app** (client_too_old) opening the install docs URL | the Attention row's subtitle shows the sentence |
| Providers | `AdwStatusPage` icon network-offline-symbolic, title "Fermix is not running", description "Providers are read from Fermix. Start it to see them.", one button **Start Fermix** | same page, title "Fermix is not responding", button **Restart Fermix** | same page, the update sentence and button | row subtitle `.error` with the sentence; never a modal |
| Chat, no transcript | same status page, description "Fermix runs in the background on this computer. Start it to chat."; composer hidden | same | same | n/a |
| Chat, transcript on screen | `AdwBanner` [1.3] "Fermix stopped" button **Start Fermix**; composer insensitive; transcript kept | banner "Fermix is not responding", **Restart Fermix** | status page replaces the composer only | ACP error → inline row in `.error` with **Try again** |

**Start Fermix** = `StartUnit("fermix.service", "replace")` on the systemd user manager over the
session bus (Flatpak finish-arg `--talk-name=org.freedesktop.systemd1`); **Restart Fermix** here is
`RestartUnit`. The button becomes an `AdwSpinner` labelled "Starting Fermix"; the app retries the socket
every 2 s for at most 15 tries, then shows "Fermix did not start" with a selectable `GtkLabel` carrying
`systemctl --user start fermix` under a dim "If that does not work", and the button returns. If systemd
refuses or the unit is unknown, the D-Bus sentence goes in the same place. `acp.sock` failing while
`daemon.sock` answers is Chat-only: status page "Chat did not answer", description "Fermix is running but
its chat service did not answer. Restarting Fermix usually fixes this.", button **Restart Fermix…**
(the §2 lifecycle route).

## 6. Chat (slice 2)

```
AdwViewStack child "chat" (AdwToolbarView; the window header gains, while Chat is selected,
  a trailing GtkButton list-add-symbolic "New conversation" Ctrl+N and AdwWindowTitle subtitle
  "ChatGPT · gpt-6-astra")
├─ content: GtkStack "empty" | "not-ready" | "conversation"
│  ├─ "empty": AdwStatusPage .compact, app icon, "Ask Fermix anything", description "Fermix keeps
│  │   what matters across conversations. This window keeps only the current one."
│  ├─ "not-ready": AdwStatusPage "Sign in to a provider to start" (description = the gating
│  │   sentences), button **Open providers**
│  └─ "conversation": GtkScrolledWindow → AdwClamp (max 720) → GtkBox vertical, spacing 12
│       user message: AdwBin .card, halign end, GtkLabel wrap, selectable, xalign 0, 12 px padding
│       Fermix message: GtkLabel wrap, selectable, halign start; AdwSpinner before the first chunk;
│         .dim-label .caption "Stopped" under a cancelled reply
│       divider: GtkLabel .dim-label .caption, centred (reconnect notice)
└─ bottom: AdwClamp → GtkBox horizontal, spacing 6, margin 12
     GtkScrolledWindow (propagate-natural-height, max ≈ 6 lines) → GtkFrame → GtkTextView
       wrap word-char. GtkTextView has no placeholder property; the empty state carries the invitation.
     GtkToggleButton audio-input-microphone-symbolic (voice; hidden until built)
     GtkButton .circular .suggested-action mail-send-symbolic ↔ .destructive-action
       media-playback-stop-symbolic
```
- **ACP.** Connect `acp.sock`, `initialize`, `session/new` on first send, then `session/prompt`; later
  sends reuse the session. Enter sends, Shift+Enter inserts a newline. The composer clears and keeps focus.
- **Streaming.** `agent_message_chunk` text is appended to the pending label, batched every 60 ms.
  The view sticks to the bottom only while the user is already within 8 px of it (vadjustment check),
  so scrolling up to read is never fought. Plain text only in slice 2.
- **Stop.** The send button becomes Stop while a prompt is in flight; Escape does the same. Stop sends
  `session/cancel`; the partial reply stays with the "Stopped" caption.
- **New conversation.** `session/cancel` if needed, clears the list to the empty state, next send calls
  `session/new`. No confirmation.
- **Permission.** `session/request_permission` renders an `AdwAlertDialog` with the agent's options
  as responses; Escape picks the option the agent marks as rejecting.
- **History** lives nowhere in the app: `loadSession` is false, nothing is written to disk, past
  conversations are not browsable, and the empty state says so. The later slot: an
  `AdwOverlaySplitView` [1.4] sidebar listing conversations saved under XDG data.
- **Reconnect.** If the ACP stream drops mid-conversation: `AdwBanner` "Chat lost its connection",
  button **Reconnect**; on success a divider "Reconnected. Earlier messages are not part of this
  conversation anymore." One reconnect per click; no automatic retry loop.
- **Errors.** A refused prompt renders the ACP error sentence as a Fermix row in `.error` with a
  **Try again** button that resends the same text.

## 7. What slice 1 deliberately leaves out

- Chat itself (slice 2), voice, channels, personalization, memory, sandbox, Doctor, Logs, tray,
  desktop notifications.
- No first-run assistant or welcome screen: Home in setup-required state with one "Open providers"
  row is the assistant.
- No provider detail page; no model or reasoning-effort pickers; no `providers.models.list`; no test
  call (`providers.probe.start`). All are Settings-section work later.
- No "Run in the background" / "Open at login" switches: systemd enable and linger, not daemon state.
- No settings sections or `settings.*` calls; the sidebar slot is reserved, nothing is built.
- No conversation history on disk, markdown rendering, attachments or tool-call display.
- No custom CSS beyond stock style classes, no custom widgets, no vendor marks the repo does not
  already ship (an `AdwAvatar` initial stands in), no animations beyond the toolkit's crossfades.
- No account name for ChatGPT: the daemon does not send one, so the row never pretends to.
- Budget: shell and sidebar 200 lines, socket client and JSON 400, connection state and polling 300,
  Home 250, Providers rows and menus 550, dialogs and jobs 350, systemd D-Bus 100. About 2.2k for
  slice 1; Chat adds about 600.

## Open questions for the owner

1. Landing page in slice 2: open on Chat whenever Fermix is ready (companion feel), or always on Home
   (the screen you liked)? This spec says Chat when ready, Home otherwise.
2. Sign-out confirmation: keep the confirm dialog, or make Sign out a one-click menu item with an undo
   toast? (The daemon revokes nothing upstream, so undo would be a fresh sign-in, not a true undo.)
3. Should the "Make primary" action also change Home's landing behaviour, or should a "Restart
   Fermix…" prompt appear right in the toast when the daemon says a restart is required?
4. Sidebar order once Chat ships: Home · Providers · Chat (setup first) or Chat · Home · Providers?
