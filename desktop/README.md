# Fermix for Linux (desktop app, fresh start)

A small GTK4 + libadwaita app, written in Rust and shipped as a Flatpak. It is a **client only**:
the engine is the host's `fermix` package, running as the `fermix.service` user unit, and the app
talks to it over `~/.fermix/daemon.sock` (management protocol v2). It holds no config and no secrets.

It replaces the earlier `App/` client beside it in this repository. That code, its private GTK
runtime and its packaging are left as they were, here and on `single-package`.

## Layout

```
core/        fermix-client: socket framing, management client, job polling rules, and every
             sentence a screen shows (view.rs). No GTK, so it builds and tests on any host.
app/         fermix-desktop: the window. Pages draw from State; app.rs/flows.rs/service.rs
             turn window actions into daemon calls.
app/marks/   vendor marks, byte for byte, with PROVENANCE.json
data/        desktop entry and icons
io.tezra.Fermix.yml   the Flatpak recipe
scripts/dev.sh        fast loop inside the GNOME 50 SDK
```

## Develop

Needs `flatpak`, the GNOME 50 SDK and the Rust extension:

```
flatpak install --user flathub org.gnome.Sdk//50 org.freedesktop.Sdk.Extension.rust-stable//25.08 org.flatpak.Builder
scripts/dev.sh run      # build (incremental) and run against your daemon
scripts/dev.sh check    # rustfmt, clippy -D warnings, tests
cargo test -p fermix-client   # the core also tests on the host
FERMIX_HOME=/nowhere scripts/dev.sh run   # see the not-running screens safely
```

Install the real app:

```
flatpak run org.flatpak.Builder --user --install --force-clean build-dir io.tezra.Fermix.yml
flatpak run io.tezra.Fermix
```

## Sandbox

`~/.fermix` is mounted **read-only**: connecting to a socket needs no write access, and the app
must never touch the config, secrets or memory files beside it (measured: a write attempt fails
with "Read-only file system"). `org.freedesktop.systemd1` is reachable so "Start Fermix" and
"Run in the background" can start, enable or stop the user unit, and `org.freedesktop.login1` so
the service can stay on after logout (linger). `xdg-config/fermix` is read-only, to know whether
the package's binding is there. The PulseAudio socket carries voice calls, microphone and speech
both: Flatpak has no finer grant. "Open at login" goes through the Background portal. The browser
opens through the OpenURI portal.

## Facts the code relies on (measured against fermix 0.11.0)

- Browser sign-in exists only for `openai_codex` and `xai`. Anthropic's ways in are importing the
  Claude Code login, a `claude setup-token`, or an API key; `auth.start anthropic` is refused.
- A ChatGPT re-sign-in leaves the provider row byte-identical, so the app acknowledges completion
  itself ("Signed in just now", a toast) instead of waiting for the row to change.
- A job does not say which provider it belongs to, so a sign-in started before the app restarted
  cannot be re-attached to its row.
