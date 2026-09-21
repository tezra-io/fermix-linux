# Acceptance runbook, Fermix for Linux

**What this is.** The gates that cannot be run unattended, written down as a
checklist a person walks in one sitting per desktop, with a column for the
evidence each one leaves behind. Everything that can run in a container already
runs in `app.yml` and in the release rail; what is left here is everything that
needs a real graphical session, a person, or a machine that reboots.

**When to walk it.** Before announcing a release. The rail can be green and every
row below still be unproven, because none of them is a code path. A desktop-only
`X.Y.Z+N` rebuild that changed only the bundled toolkit still needs rows 6, 13,
14 and 15, because a toolkit bump is exactly the change that moves rendering.

**Where.** One GNOME host, one Plasma host and the Pop!_OS 22.04 machine, on a
cloud desktop instance or spare hardware. Pop!_OS 22.04 is its own row because
it is the owner's own machine and it is the oldest supported target: its host
GTK is 4.6, which is the exact claim the private runtime exists to make. There
is no local virtual machine capacity on the development machine, which is the
constraint that keeps this list at twenty-two rows rather than forty: a gate that
can be neither automated nor run by hand in a sitting will not be run at all.

**How to record a row.** Fill in the date, the desktop, the two versions, and the
evidence: a file name in `docs/design/captures/`, a command transcript, or a
screen recording. A row with no evidence is Pending, whatever anybody remembers.

---

## Before the sitting

| | Fact | Value |
|---|---|---|
| | Desktop and version | GNOME __ / Plasma __ / Pop!_OS 22.04 |
| | Session type | Wayland / X11 |
| | The host's own GTK and libadwaita | __ / __ |
| | The package's GTK and libadwaita | read from `/usr/share/doc/fermix-desktop/runtime-manifest.json` |
| | `fermix-desktop --version` | |
| | `fermix --version` | |
| | Package file installed | |
| | Its sha256 | |

Install from the release page exactly as `INSTALL.md` on it says: one package,
one command. An install done any other way proves something else.

The host toolkit row is recorded because it is the point: on Pop!_OS 22.04 and
on Ubuntu 22.04 the host carries GTK 4.6 and the window draws with the 4.16 the
package brought. `ldd /usr/lib/fermix-desktop/bin/fermix-desktop` should show
every toolkit library resolving inside `/usr/lib/fermix-desktop/lib` and nothing
GTK-shaped coming from `/usr/lib`.

---

## The twenty-two (M38 section 13.2, amended, plus the secret-store and tray rows)

### 1. Wayland truth

The Computer pane reports the actual unsupported sidecar state and offers no
capture or input permission action. Opening the pane prompts for nothing.

| Result | Evidence | Date | Host |
|---|---|---|---|
| Pending | | | |

Look for: the pane's own sentence naming this session, no dialog on render, and
no button that would ask for a permission this version cannot hold.

### 2. Session transitions on the real host

Start the daemon through linger before login. Then log in, unlock the keyring,
lock it, replace the Secret Service owner, and log out. New observations follow
those changes without restarting the engine, and nothing attributes the window's
own context to the daemon.

| Result | Evidence | Date | Host |
|---|---|---|---|
| Pending | | | |

### 3. Tray present

A desktop with a live `org.kde.StatusNotifierWatcher` whose
`IsStatusNotifierHostRegistered` is true. v1 registers no item at all, so the
assertion is that nothing appears and every action is still reachable from the
window.

| Result | Evidence | Date | Host |
|---|---|---|---|
| Pending | | | |

### 4. Tray absent

Vanilla GNOME with no AppIndicator extension: no item is registered, nothing is
pretended, and every action remains reachable from the window.

| Result | Evidence | Date | Host |
|---|---|---|---|
| Pending | | | |

### 5. Notification actions and lifetime

Deliver an attention-change notification while the window is open. Close it.
Then activate an already-posted notification and confirm the installed
application opens Home. An exited client posts nothing and is expected to.

| Result | Evidence | Date | Host |
|---|---|---|---|
| Pending | | | |

### 6. System accent and colour scheme

Change both in GNOME Settings and in Plasma's System Settings, in both
directions, and confirm the window follows through the appearance portal.

| Result | Evidence | Date | Host |
|---|---|---|---|
| Pending | | | |

The application sets no colour at all, so this is a test of that fact rather than
of a feature.

### 7. Screen reader traversal

Orca over every persistent surface, plus an Accerciser tree dump asserting that
every interactive node has a role and a name.

| Result | Evidence | Date | Host |
|---|---|---|---|
| Pending | | | |

### 8. Reduced motion, high contrast and text scaling

Toggle each system preference and confirm the application responds, including
text scaling at the value GNOME's Large Text sets.

| Result | Evidence | Date | Host |
|---|---|---|---|
| Pending | | | |

### 9. Single-instance raise

A second launch from the desktop entry raises the existing window rather than
starting a second process, from a cold session as well as a warm one. The cold
path is the one that matters here: it runs through D-Bus activation and the
packaged unit, which is the only place `app-io.tezra.Fermix.service` and
`io.tezra.Fermix.service` are both exercised.

| Result | Evidence | Date | Host |
|---|---|---|---|
| Pending | | | |

Check `systemctl --user status app-io.tezra.Fermix.service` afterwards: the
window should be running inside that unit, not as a child of the shell.

### 10. Desktop setup end to end

The setup assistant from a fresh home to a ready daemon on a real session,
including the browser handoff for an OAuth provider.

| Result | Evidence | Date | Host |
|---|---|---|---|
| Pending | | | |

### 11. Software-centre discovery and updates

On a desktop with no Fermix installed, follow the repository instructions,
refresh GNOME Software or KDE Discover, find the entry and install it. Then
verify a later package update and its release notes.

| Result | Evidence | Date | Host |
|---|---|---|---|
| Pending | | | |

This row cannot pass until the repository service exists; until then it is run
against a local file install and the metainfo's rendering is what is judged.

### 12. Portal connection identity

A packaged application has unit-derived identity even with the host Registry
interface absent or erroring. An explicitly direct-launched connection can
register before its first portal call, and a different connection does not
inherit that registration. A truly unverified identity reports unknown, and is
never asserted to be an empty or shared application id.

| Result | Evidence | Date | Host |
|---|---|---|---|
| Pending | | | |

### 13. Visual quality review

Inspect the reference views at actual size on both desktops, against
`docs/design/LINUX_DESIGN_SYSTEM_REDLINES.md` sections 3 and 6. Record the
fixture, the environment and the decision beside each capture in
`docs/design/captures/INDEX.md`. Verify hierarchy, readable density, artwork,
alignment and interaction continuity, including Settings inside the main window.
Exercise reduced motion during navigation and confirm that polling does not
replay decorative entrances.

| Result | Evidence | Date | Host |
|---|---|---|---|
| Pending | | | |

**Every capture in `docs/design/captures/` currently says `pending` in its
decision column.** This row is where that changes: a capture that exists is not a
capture that was accepted.

### 14. A CJK input method, on each session type

Install fcitx5 or ibus with a CJK engine, and type Chinese, Japanese or Korean
into every field the window has: the home path, the search entries, the About
you fields. Do it once in a Wayland session and once in an X11 session, on the
same host.

| Result | Evidence | Date | Host | Session |
|---|---|---|---|---|
| Pending | | | | Wayland |
| Pending | | | | X11 |

This row exists because the package carries its own GTK, and a host fcitx5 or
ibus GTK4 input module links the host's GTK and therefore cannot be loaded into
this process. What is expected, and what the row is checking rather than hoping:

- on **Wayland**, `text-input-v3` puts input method handling in the compositor,
  so fcitx5 and ibus both work with no module at all and the preedit appears
  over the window;
- on **X11**, there is no such protocol, so an fcitx5 user falls back to XIM.
  XIM works and is worse: the preedit is drawn over the window rather than in
  place, and a composed string still reaches the field. A row that records XIM
  behaving that way is a pass; a row where nothing reaches the field is a
  failure.

The package builds GTK with its own `ibus` module inside the private prefix and
compiles in the `simple` and `none` contexts, so an ibus user has the private
module on both session types.

### 15. The floor host, as a person uses it

On Pop!_OS 22.04, with its GTK 4.6 and its COSMIC-on-GNOME session: install the
deb from the release page, open the window from the launcher, walk setup end to
end, change the system colour scheme in both directions, and open a link from
inside the window.

| Result | Evidence | Date | Host |
|---|---|---|---|
| Pending | | | |

Four things are being watched, and each is a way this design fails quietly on
the oldest target:

- the window draws at all, which is the private GTK 4.16 running against a host
  at 4.6;
- the colour scheme follows, which on this session comes through the appearance
  portal and, where that is absent, through the bundled dconf GSettings backend
  reading the host's `org.gnome.desktop.interface`;
- the browser that opens from a link starts with the environment this process
  started with, not with `GSETTINGS_SCHEMA_DIR` pointing at a private GTK 4.16
  schema directory. Check it: `tr '\0' '\n' < /proc/<browser pid>/environ | grep GSETTINGS`
  must print nothing that this window put there;
- the icons are the host's Adwaita where the host has one, because the bundled
  copy is appended to the search path and is a floor rather than an override.

### 16. A locked keyring, answered

The row the owner's own desktop failed on. Log in by fingerprint or with
automatic login, so the login collection is never unlocked by a password, and
confirm the starting state before touching the window:

```sh
busctl --user get-property org.freedesktop.secrets \
  /org/freedesktop/secrets/collection/login org.freedesktop.Secret.Collection Locked
```

`b true` is the precondition. Then save a key in the window — an API token in
setup or in Settings.

| Result | Evidence | Date | Host | Session |
|---|---|---|---|---|
| Pending | | | | |

Record each of these, not a general impression:

- the **"Your Keyring Is Locked"** dialog appears, rather than a spinner, an
  error, or a claim that there is no keyring;
- **Unlock Keyring** raises the system password prompt, and typing the account
  password completes it;
- the save then succeeds and the result names `store: "keyring"`;
- the `Locked` property above now reads `b false`.

The failure this catches is the one already seen: the engine inferred a lock
from a helper timeout, so a locked keyring reported `timeout` and the person was
told nothing true. The contract reads `Locked` before the write.

**If the save is refused, look in the keyring anyway** —
`secret-tool search --all service fermix`. A refusal that has nevertheless
stored the value has happened here (see row 20), and a message that says nothing
was saved is not evidence that nothing was.

**A locked keyring fails in two different shapes, and which one you get is
decided by the session rather than by the keyring.** `secret-tool` asks
gcr-prompter for the password dialog, and gcr-prompter needs a display. With
one, it starts, puts the dialog on the screen, and `secret-tool` blocks until
someone answers — so an engine that infers the state from the helper sees a
timeout. Without one, the prompter fails to activate and `secret-tool` exits 1
at once with *Cannot create an item in a locked collection* — so the same engine
sees a clean failure and calls it `unavailable`. Same helper, same collection,
two answers. Measured here on 2026-09-20.

That is why this row is walked on a real session: a container gets the second
shape only, and two people reasoning from different machines will reach
confidently opposite conclusions about what a locked keyring does. If you are
ever interpreting a keyring failure by its shape, ask first whether a prompter
could have appeared.

**The two shapes only apply to an engine that actually runs the helper.** An
engine that reads the collection's `Locked` property and refuses on it never
invokes `secret-tool` at all, so neither shape occurs and the session cannot
influence what it reports — a wrong answer from such an engine is a mapping
fault rather than a headless artefact. `scripts/keyring_smoke.sh` is how the two
are told apart: it puts a logging `secret-tool` ahead of the real one on the
unit's PATH and counts the store invocations. Applying the two-shapes rule to an
engine that never ran the helper will have you concluding a machine is headless
when it is not.

**From the shipping engine on, that is the case you are in.** Confirmed by the
engine author on 2026-09-20: this engine measures the property and does not
consult the helper, and `scripts/keyring_smoke.sh` records zero store
invocations on its locked arm. So its `unavailable` is unconditional — it says
nothing whatever about whether a session or a display was present, and cannot be
read as evidence either way. Do not invert the paragraph above into "no
prompter appeared, so the machine must be headless": on this engine no prompter
appears on any machine, because nothing ever asks for one.

**One case the dialog cannot distinguish, worth recording if you meet it.** The
engine reports `unavailable` both when there is no Secret Service and when it
cannot read the `Locked` property on a machine that has one. So a
*"This Computer Has No Keyring"* dialog on a machine where
`busctl --user list | grep org.freedesktop.secrets` shows a live service is not
a lie about the keyring — it is the engine saying it could not measure it. Note
it here with the `busctl` output rather than filing it as a wrong message.

### 17. A locked keyring, walked away from

The same starting state as row 16, but do nothing when the dialog appears. Leave
the machine for two minutes.

| Result | Evidence | Date | Host | Session |
|---|---|---|---|---|
| Pending | | | | |

- at about **90 seconds** the call answers `locked` rather than hanging;
- the same dialog returns with **both** actions still offered, so the person who
  came back has the same two ways forward they had before;
- nothing was stored anywhere, and no file appeared under the Fermix home;
- the app's own deadline (about 105 s) did not fire first — the person sees the
  engine's typed reason, not a client timeout. If they see a generic timeout
  instead, the two caps have drifted and this row fails.

### 18. Store on this computer

From the locked dialog, choose **Store on This Computer**.

**Check the way back exists before you take this choice.** The reversal is the
engine's `secret.migrate_to_keyring`, and an engine without that verb makes the
file store one-way: the value cannot be read back out for retyping, so the only
route home is typing every key again from wherever it came from. Confirm the
verb is in the build's method list before choosing it — `hello` over the
management socket returns `capabilities.methods` — and on a build that lacks it,
do not accept the offer on a machine holding credentials you cannot re-obtain.
That is a real hazard on an interim or a test build, where the app may offer a
choice the engine cannot undo.

| Result | Evidence | Date | Host | Session |
|---|---|---|---|---|
| Pending | | | | |

- the save succeeds and the result names `store: "file"`;
- on disk, under `<FERMIX_HOME>/secrets/`: `ls -ln` shows the directory `0700`
  and the file `0600`, both owned by the running user;
- **Settings > Permissions** shows *Where keys are stored / Stored on this
  computer*;
- `secrets.store` in the settings file reads `"file"`.

The consent is the choice: there is no automatic fallback, so a file store that
appears without the person choosing it is a failure of this row even if the save
succeeded.

**Read that row where you stand, immediately after the action, before
navigating anywhere.** The row is drawn from a snapshot, and one failure it has
to catch is that snapshot not being re-read after the action. Found by reading
code on 2026-09-20, in a build that had passed every gate: after a consented
`secret.set` the row could go on saying *Stored in your desktop keyring* while
the key had just gone to the file store. That is the row telling the owner the
opposite of what just happened, at the one moment they made a deliberate
decision about their own key.

That was fixed the same day, and the fix is what makes reading in place the
right test rather than a precaution. Both paths that move where secrets live now
re-read `setup.state.get` themselves before the row redraws, and that re-read
cannot be dropped by the read ceiling: under read pressure it retries, bounded
at five seconds. So the row is expected to be correct the instant the action
completes, with no navigation needed, and on a loaded machine within five
seconds of it. Read it there, and if it is still wrong after that, that is the
bug.

Knowing the bound is what lets you judge the row rather than guess at it. A
reviewer who reads instantly on a busy machine and sees the old value has caught
the retry in flight, not a defect; one who waits past five seconds and still
sees it has caught a defect. Record which of the two you did, because the same
observation means opposite things depending on it.

Do not navigate first and then read. As the code stood before the fix, none of
the three obvious recoveries corrected the row — leaving the pane and returning,
closing the Settings window and reopening it, or any action short of restarting
the process — so a row read after wandering about tells you nothing useful about
either the old behaviour or the new. If you did navigate before reading, record
what you did rather than a verdict.

**The Settings row is the one line here that nothing automated can see, so read
it rather than glance at it.** Every container gate asserts on the management
wire, the two stores, the journal and the daemon; none of them opens a window.
A row that draws nothing — because the app reads a field the engine has stopped
sending, which has already happened once to `setup.state.get`'s store field —
looks exactly like a row nobody checked. The engine can be answering perfectly
while the window says nothing at all, and this line is the only place that
difference is caught before the owner meets it.

**There is now a second reader, and it is conditional.** The capture
`settings_secret_store_keyring` draws this row from the vendored
`fixtures/success.jsonl` golden rather than from hand-written app-side data —
verified with slice 5 on 2026-09-20, scenario `default`, whose `overrides.jsonl`
holds no records at all, so `setup.state.get` is answered straight from the
golden. And the app hides the group, the row and the caption outright when the
row is missing (`draw_store`, `src/ui/settings/permissions.rs:201`), so a field
the engine stopped sending does not render as a wrong value — a whole group
disappears from the pane, which is obvious at a glance.

**The condition is the re-take, and nothing enforces it.** A capture taken
before the field moved shows the row perfectly and proves nothing; it is
precisely the artefact that looks reviewed. So the second reader is not the
capture, it is the capture *plus* being re-taken against the new golden when
the contract is vendored. Nothing in the repo forces that today. If you are
walking this row after a contract change, check that the captures were re-taken
since — and if they were not, this line is carrying the failure alone again,
exactly as it did before the capture existed.

That the old `settings_permissions` capture carried this row's title while
showing none of its contents, and so would have satisfied a reviewer skimming
for whether the row was there, is the same failure one level down. It was found
by re-taking the capture rather than by looking at the existing one and judging
it fine.

### 19. Use the keyring instead

With a value in the file store from row 18, unlock the keyring and choose **Use
the Keyring Instead**.

| Result | Evidence | Date | Host | Session |
|---|---|---|---|---|
| Pending | | | | |

- the result lists what moved;
- the file under `<FERMIX_HOME>/secrets/` is **gone**, and the directory is
  empty;
- the Settings row now reads the keyring;
- `secrets.store` is cleared, not left saying `"file"`.

Each value is written and read back before its file copy is deleted, so a
half-migrated state is a failure rather than a caveat: if any file remains while
the setting says keyring, record it.

**Same rule as row 18: read the row where you stand, immediately after the
action.** This is the other half of the same defect and the more misleading
half — after a successful migration the row could go on saying *Stored on this
computer* and go on offering a way back that has already been taken. An owner
who believed it would think the migration failed and could reasonably take the
offer again, which is a worse outcome than a row that says nothing. The
migration path now re-reads the snapshot before the row redraws, so the row is
expected to be right the instant the migration returns; read it there, for the
reason given in row 18.

### 20. An unlocked keyring, ordinary save

Log in with a password, so the login collection unlocks at login, and save a key
normally.

| Result | Evidence | Date | Host | Session |
|---|---|---|---|---|
| Pending | | | | |

- **no dialog at all**;
- the save **succeeds in the window**, `store: "keyring"` — not an error, not
  "something went wrong";
- `journalctl --user -u fermix` contains no `Management route failed` line for
  `secret.set`;
- the key is really there: `secret-tool search --all service fermix` shows it;
- the engine is still the same process — `fermix service status --json` reports
  `active` with the same `pid` and `restart_count` as before the save.

**The first three lines are the row; the last one is not enough on its own, and
that is measured rather than supposed.** Run against the package `e6d4b266…` on
2026-09-20, an unlocked keyring produced this: the keyring write succeeded and
returned `{:ok, ""}` to a write log with no clause for it, so the route raised,
the caller was handed `internal_error` — and the daemon never died. `pid` and
`restart_count` were identical either side of the failure. A row that checked
only those would have called that machine healthy.

The credential was in the keyring the whole time. So the person is told the save
failed while their key is stored: they retype it, or they choose the file store
instead, and now the same credential is in two places and they know about one.
That is why this row reads the value back rather than trusting the message.

`scripts/keyring_smoke.sh` automates the parts of this that need no desktop —
the unlocked save and the absent-store case — and refuses on exactly the
failure above. It cannot replace this row, because a container has no dialog to
show and nobody to read it.

### 21. No Secret Service at all

A session with no Secret Service running — no gnome-keyring-daemon, no KWallet,
no KeePassXC. Confirm it first:

```sh
busctl --user list | grep org.freedesktop.secrets   # expect no output
```

| Result | Evidence | Date | Host | Session |
|---|---|---|---|---|
| Pending | | | | |

- the **"This Computer Has No Keyring"** dialog appears;
- the reason is `unavailable`, never `locked`;
- doctor and readiness report `availability: "unavailable"`, and `store` is not
  claimed to be `"keyring"`.

The distinction between this row and row 16 is the whole point of reading the
property: absent and locked are different machines and different remedies, and
one message for both is what this work removes.

### 22. The status icon

**This row is void unless the desktop has a status area, so establish that
first.** Pop!_OS and Ubuntu have one by default. Fedora Workstation GNOME has
none unless someone installed an extension for it, and on such a machine an
absent icon is correct behaviour rather than a defect.

```sh
busctl --user list | grep StatusNotifierWatcher   # must name an owner
```

If nothing owns it, record the row **not applicable on this host** and go
straight to the last clause, which is the one that still applies.

| Result | Evidence | Date | Host | Session |
|---|---|---|---|---|
| Pending | | | | |

- within **five seconds** of launching Fermix an icon is visible in the top bar,
  read where you stand without navigating anywhere. Five seconds rather than
  "eventually": registration is immediate, and a panel that has not drawn it by
  then is not going to;
- **any** click opens the menu, left or right alike — there is deliberately no
  difference between them, so a right-click that does something else is a defect;
- the menu reads, top to bottom: a greyed line saying what the daemon is doing,
  a separator, **Back to Fermix**, **Settings**, **Run Doctor**, a separator,
  **Restart Fermix**, a separator, **Quit**. The first line is not clickable, and
  that is deliberate — it is a fact, not a control;
- **with no window open**, which is the case the icon exists for: Back to Fermix
  opens a window on Home, Settings on Settings, Run Doctor on Doctor. A row that
  opens the wrong page is a defect; a row that does nothing at all is the defect
  this row most exists to catch;
- closing the last window **leaves the icon** and Fermix keeps running —
  `pgrep -f fermix-desktop` finds it — and there is **no** dash or task bar entry
  while no window is open, because an application holding itself open with no
  window contributes none;
- **Quit from the menu ends the process**: after clicking it, `pgrep -f
  fermix-desktop` finds nothing.

Quit is the clause to be most careful about. With the icon holding the
application open, it is the only thing that ends it, so a Quit that leaves a
process behind leaves one the person cannot see and did not agree to.

**If there is no icon, this row is judged on the journal rather than on the
absence.** `journalctl --user -b | grep -i fermix` must carry exactly one of:

| Sentence | Verdict |
|---|---|
| *this desktop has no status area* | correct, not a defect — see the precondition |
| *the status area refused the icon* | a defect, and the sentence carries the reason |
| *no status icon: the session bus could not be reached* | a defect |

**Silence is itself a defect.** An icon that is absent with nothing said is the
failure this feature is most prone to, and the code is written so that all three
cases say something. A reviewer who finds no icon and no sentence has found a
defect, not an inconclusive row.

**Registration and drawing are two different events, so do not report the wrong
one.** Our side can register successfully and the panel still not draw us — that
is documented behaviour of the GNOME extension when it cannot read our menu, and
it is the reason the journal clause exists. If there is no icon, record which
sentence the journal carried; do not record "registration failed", which is a
claim about our side that the absence alone does not support.

**What the automated gate already covers, so this row need not.**
`scripts/tray_smoke.sh` exercises the whole D-Bus path against a stub host on a
private bus: registration, the properties a panel reads, the menu layout, a click
on every row reaching its action, and negative arms for hover-does-not-fire,
no-watcher and a malformed layout. It proves the protocol a host speaks, and it
cannot prove GNOME draws us, because no container has a panel. The protocol is
settled; the drawing is what you are here for.

That distinction is not academic: the gate caught a reply that every pure-value
test had agreed with, a bare array where D-Bus requires a tuple, which a real bus
refused outright — the panel would have received an error instead of a menu. A
test that never spoke to a bus could not see it, and neither could this row.

---

## The Stage 0 rows that need a real host (M38 section 1.4)

The container lane covers most of Stage 0. These are the ones it structurally
cannot, because a container does not reboot its host and has no real login
session.

### S1. Reboot and custom home

Linger starts the bound home before login. Then disable, quit the window,
relaunch it and enable again: the same home is used, and it is a home whose path
contains a space and a percent character.

| Result | Evidence | Date | Host |
|---|---|---|---|
| Pending | | | |

The container half of this round trip runs in the install smoke, which installs
into `/home/test/fermix home`. What is left here is the reboot.

### S2. Exact GUI artifact on the floor

Run the released binary, without rebuilding it, on a host whose own GTK is below
the floor and on one above it. Audit the imported symbols against the floor.
Confirm a host below the **glibc** floor refuses the package install rather than
accepting it and failing at exec: openSUSE Leap 15.6, whose glibc is 2.31, is
the host that proves it, and the refusal should come from zypper naming
`libc.so.6(GLIBC_2.34)` rather than from anything Fermix runs.

| Result | Evidence | Date | Host |
|---|---|---|---|
| Pending | | | |

This is the row `scripts/package_dependencies.py` cannot stand in for: that gate
proves nothing in the package links outside the private prefix except to a
declared host relation, and this one proves the result runs where the
declaration says it will.

```sh
objdump -T /usr/lib/fermix-desktop/bin/fermix-desktop | grep -E 'GTK_4\.|ADW_1\.' | sort -u
ldd /usr/lib/fermix-desktop/bin/fermix-desktop | grep -v fermix-desktop
```

The second command is the whole claim in one line: what it prints is the host
half of amendment section 4.1 and nothing else, no GTK, no GLib and no pango.

### S3. Socket and listener boundaries, across a reboot

Two accounts with two persisted listener ports survive a reboot and serve their
own endpoints. An overlong socket address refuses before bind rather than exiting
with an unhandled error.

| Result | Evidence | Date | Host |
|---|---|---|---|
| Pending | | | |

---

## What a Pending row costs

A Pending row is not a blocked release on its own; an unproven row that is
recorded as proven is. If a row cannot be walked this time, leave it Pending and
say so in the release notes rather than passing it on a recollection.
