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
- Commit: `6a7472361a437820fa520fcc476002e0fd1f17b0` (2026-11-02, branch `dev`), whose last
  change to `priv/management` is `352ae545` ("fix(home): a move back never leaves a screen with
  nothing to press or a server it cannot restore"). Since v0.17.0 (`b9dce743`), all within
  version 2: the `home.*` methods (the home's status with its Always On `summary`, `disk`,
  `version` and `update_ceiling`, recovery snapshots, enrollment, candidates, the move,
  `home.provision.options` and `home.act`), the `snapshots` section under pane `always_on`,
  `settings.sections.primary_missing`, a paired device's `kind`, the codes `primary_unreachable`,
  `device_class_method`, `unconfirmed` and `changed`, the plugin status `reinstall_failed`, the job
  kind `home_provision`, a cue watcher's `follow_up` with `always_allowed`, and
  `cues.watchers.always_allow.revoke`. Every Always On step is one `home.act {action, key,
  input}` the engine runs, answered `{summary, result}` or `changed` with its sentence alone; every
  summary action is one record (`id`, `label`, `needs`, `confirm`, `url`, `key`). Since the
  previous pin (`e1b7cbbb`): Move back, the home's way back from its server to the computer it
  left (the actions `move_back`, needing nothing, and `restore_here`, needing the
  `recovery_passphrase` the copy it restores was written for and asked first; the problems
  `server_retired` and `return_failed`; the notice `back:<epoch>`; `on`'s third action; the
  `Backup on this Mac` and `Last seen` lines beside a restore; Move now's confirm during a move
  back; `home.enroll.withdraw {kind}`); `home.status`'s `update_ceiling`, the version a server's
  nightly update installs; `computer_use.permissions.get`'s optional `platform`, and on Linux
  `display_server` and the doctor's `rows`, with a compatibility golden of a daemon from before
  them; and the rule that a field a later protocol-2 engine adds to a v2 method is optional too.
- Protocol version: 2, daemon window 1..2.
- `CHECKSUMS.txt` holds the SHA-256 of each file, in `sha256sum` format.

Never edit these files by hand. To take a new engine contract, copy the six files again from a
committed engine, rewrite `CHECKSUMS.txt` with `sha256sum PROTOCOL.md protocol.schema.json
fixtures/requests.jsonl fixtures/success.jsonl fixtures/errors.jsonl fixtures/compatibility.jsonl`,
update the commit above, and run `cargo test -p fermix-client`: the golden fixtures then say whether
the app still decodes what the daemon sends.
