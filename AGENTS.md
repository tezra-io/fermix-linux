# Fermix for Linux

A Rust GTK4 + libadwaita desktop app (gtk4-rs 0.11, libadwaita 0.9, the GNOME 50 Flatpak runtime),
in `desktop/`. It is only a client of the host's `fermix` daemon, which runs as the `fermix.service`
user unit:

- settings, sign-in and status: `~/.fermix/daemon.sock`, management protocol v2 (a 4-byte big-endian
  length, then JSON);
- chat: `~/.fermix/acp.sock`, a `{"fermix_bridge":1,...}` handshake line, then ACP v1 NDJSON;
- voice: `~/.fermix/realtime.sock`, NDJSON, protocol 2, with PCM16 audio at 24 kHz.

This is the repo's only agent-instruction file. Never add a `CLAUDE.md`, `.claude/CLAUDE.md` or
`CLAUDE.local.md`.

## Layout
```
desktop/core/       fermix-client: the wire, the state rules and every sentence a screen shows; no GTK
desktop/app/        fermix-desktop: the window, which draws what core decides
desktop/app/marks/  vendor marks and the Fermix brand files, byte for byte, with PROVENANCE.json
desktop/docs/       the design record the code cites (design_final.md, spec_voice.md)
desktop/data/       desktop entry and icons
desktop/io.tezra.Fermix.yml   the Flatpak recipe
desktop/scripts/dev.sh        build, run and check inside the GNOME 50 SDK
```

## Working rules
- The daemon decides and the app renders. A state the daemon publishes is never derived here, and
  its sentences are shown as it wrote them.
- Pure logic lives in `core` with its tests in `core/tests/`, written first. `app` only draws.
- Vendor marks ship byte for byte with their provenance; nothing is redrawn or recoloured beyond
  what a record permits.
- Done means `desktop/scripts/dev.sh check` is green: rustfmt, `clippy --workspace --all-targets
  -D warnings`, and every test. Zero warnings.
- Tests and harnesses never touch the host: no real daemon socket, nothing written under
  `~/.fermix`, no real microphone or speakers (`FERMIX_AUDIO=test`), and no portal dialogs
  (`GDK_DEBUG=no-portals`).
- No AI attribution anywhere. Never commit or push unless the owner says so.
