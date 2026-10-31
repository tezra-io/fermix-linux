# Where this contract comes from

These six files are the engine's management contract, copied byte for byte:

| file here | engine path |
|---|---|
| `PROTOCOL.md` | `apps/fermix_core/priv/management/PROTOCOL.md` |
| `protocol.schema.json` | `apps/fermix_core/priv/management/protocol.schema.json` |
| `fixtures/requests.jsonl` | `apps/fermix_core/priv/management/fixtures/requests.jsonl` |
| `fixtures/success.jsonl` | `apps/fermix_core/priv/management/fixtures/success.jsonl` |
| `fixtures/errors.jsonl` | `apps/fermix_core/priv/management/fixtures/errors.jsonl` |
| `fixtures/compatibility.jsonl` | `apps/fermix_core/priv/management/fixtures/compatibility.jsonl` |

- Repository: fermix (the engine)
- Draft: copied on 2026-10-28 at 03:39 EDT from the engine's uncommitted working tree on `dev`
  (HEAD `9d4e1ec252b8e7bc935d9f220a293d46efd8a763` plus the uncommitted M75 Always On work and
  its whole fix wave, the export final for that round), for review only. No engine commit
  publishes these bytes, and `release.yml` refuses to build a release while this line is here.
  Re-vendor the six files from the engine commit that publishes them, name that commit below,
  and delete this line.
- Commit: none yet. The last committed contract this app vendored was the engine v0.17.0
  (`b9dce743b4f31dfcfd27874d9f4baf43d6a096d3`; last change to `priv/management`
  `fe7fe1c01fa5313a70ee3049470f853f546f6883`). Since v0.17.0, all within version 2 and with no
  method removed: the `home.*` family (`home.status` with `secrets_missing`, `primary_home` and
  `primary_missing`; `home.snapshots.status`, `.run` and `.passphrase`; `home.enroll.open`;
  `home.candidates.list`; `home.move.readiness`; `home.move.start` with `keep_as_desktop`), the
  move phases `committed` and `undoing`, `home.status.primary.status`, the `engine` and
  `snapshots` sections under pane `always_on`, `settings.sections` with `primary_missing`,
  `device.grants.list` and `device.grants.remove`, `mobile.devices.list` rows with `kind` and
  `executor`, `mobile.devices.set_executor`, the browser sign-in and hand-off
  (`browser.signin.open`, `browser.handoff.send`, `browser.handoffs.list`, `.wipe`), the device
  role's forwarding of primary-class methods (the home family's recovery, enrollment and move
  among them) with its forward deadlines and composed answers, and the error codes
  `primary_unreachable`, `device_class_method` and `unconfirmed`.
- Protocol version: 2, daemon window 1..2.
- `CHECKSUMS.txt` holds the SHA-256 of each file, in `sha256sum` format.

Never edit these files by hand. To take a new engine contract, copy the six files again from a
committed engine, rewrite `CHECKSUMS.txt` with `sha256sum PROTOCOL.md protocol.schema.json
fixtures/requests.jsonl fixtures/success.jsonl fixtures/errors.jsonl fixtures/compatibility.jsonl`,
update the commit above, and run `cargo test -p fermix-client`: the golden fixtures then say whether
the app still decodes what the daemon sends.
