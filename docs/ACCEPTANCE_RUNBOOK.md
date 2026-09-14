# Acceptance runbook, Fermix for Linux

**What this is.** The gates that cannot be run unattended, written down as a
checklist a person walks in one sitting per desktop, with a column for the
evidence each one leaves behind. Everything that can run in a container already
runs in `app.yml` and in the release rail; what is left here is everything that
needs a real graphical session, a person, or a machine that reboots.

**When to walk it.** Before announcing a release. The rail can be green and every
row below still be unproven, because none of them is a code path.

**Where.** One GNOME host and one Plasma host, on a cloud desktop instance or
spare hardware. There is no local virtual machine capacity on the development
machine, which is the constraint that keeps this list at thirteen rows rather
than forty: a gate that can be neither automated nor run by hand in a sitting
will not be run at all.

**How to record a row.** Fill in the date, the desktop, the two versions, and the
evidence: a file name in `docs/design/captures/`, a command transcript, or a
screen recording. A row with no evidence is Pending, whatever anybody remembers.

---

## Before the sitting

| | Fact | Value |
|---|---|---|
| | Desktop and version | GNOME __ / Plasma __ |
| | Session type | Wayland / X11 |
| | GTK and libadwaita | __ / __ |
| | `fermix-desktop --version` | |
| | `fermix --version` | |
| | Package files installed | |
| | Their sha256 | |

Install from the release page exactly as `INSTALL.md` on it says, with both
packages in one command. An install done any other way proves something else.

---

## The thirteen (M38 section 13.2)

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

Run the released binary, without rebuilding it, on a host at GTK 4.16 and
libadwaita 1.6, and on a newer one. Audit the imported symbols against the floor.
Confirm a host below the floor refuses the package install rather than accepting
it and failing at exec.

| Result | Evidence | Date | Host |
|---|---|---|---|
| Pending | | | |

This is the row the declared-against-derived dependency check cannot stand in
for: that check proves the declaration covers what the binary needs, and this one
proves the binary runs where the declaration says it will.

```sh
objdump -T /usr/bin/fermix-desktop | grep -E 'GTK_4\.|ADW_1\.' | sort -u
```

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
