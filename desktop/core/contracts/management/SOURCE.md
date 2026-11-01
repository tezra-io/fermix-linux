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
- Commit: `bcae16b9b983da22831a55cd01fc41ed4ac2f558` (2026-10-31, branch `dev`), which is also
  the last commit to touch `priv/management` ("feat(web): adding a source on the Cues tab lists
  every choice at once, as the Mac app does, with nothing folded under Something else"). Since
  v0.17.0 (`b9dce743`), all within version 2: the sixteen `home.*` methods (the home's status with
  its Always On `summary`, `disk` and `version`, recovery snapshots, enrollment, candidates, the
  move, and the seven `home.provision.*` methods of the cloud provisioner, `providers` among them),
  the `snapshots` section under pane `always_on`, `settings.sections.primary_missing`, a paired
  device's `kind`, the codes `primary_unreachable`, `device_class_method` and `unconfirmed`, the
  plugin status `reinstall_failed`, the job kind `home_provision`, a cue watcher's `follow_up`
  with `always_allowed`, and `cues.watchers.always_allow.revoke`. Since the previous pin
  (`42f02f6f`): `home.provision.providers`; `provider` on `home.provision.connect`, `options` and
  `create` and in the provisioner's view; locations and `next_location` as open strings; a
  resource's `id` as a string; the summary action `open_console` in place of `open_hetzner`; the
  failure reasons `rate_limited`, `unreachable` and `other`; the provider-neutral owner-decision
  sentence; that `home_status_changed` (companion protocol 6) says when the summary changed;
  `cues.watchers.always_allow.revoke` and the watcher row's `follow_up.always_allowed` in place of
  `mode` and `approved`; and the cue screen's wording.
- Protocol version: 2, daemon window 1..2.
- `CHECKSUMS.txt` holds the SHA-256 of each file, in `sha256sum` format.

Never edit these files by hand. To take a new engine contract, copy the six files again from a
committed engine, rewrite `CHECKSUMS.txt` with `sha256sum PROTOCOL.md protocol.schema.json
fixtures/requests.jsonl fixtures/success.jsonl fixtures/errors.jsonl fixtures/compatibility.jsonl`,
update the commit above, and run `cargo test -p fermix-client`: the golden fixtures then say whether
the app still decodes what the daemon sends.
