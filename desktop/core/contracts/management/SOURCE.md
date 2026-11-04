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
- Commit: `6d60af55b4e5358686314cc1f5da6f518c2313bd` (2026-11-04, branch `m75/cut-provisioning`,
  which reaches `dev` by fast-forward with the same commit once its CI passes; `dev` was at
  `3b306ef1` when these were copied), whose last change to `priv/management` is `5178ebf8`
  ("fix(home): a move away from a home removes the record of the move back that ended there, so no
  later move's progress, move back or notice is drawn from it"). Since v0.17.0 (`b9dce743`), all
  within version 2: the `home.*` methods (the home's status with its Always On `summary`, `disk`,
  `version` and `update_ceiling`, recovery snapshots, enrollment, candidates, the move and
  `home.act`), the `snapshots` section under pane `always_on`, `settings.sections.primary_missing`,
  a paired device's `kind`, the codes `primary_unreachable`, `device_class_method`, `unconfirmed`
  and `changed`, the plugin status `reinstall_failed`, the job kind `home_restore`, a cue watcher's
  `follow_up` with `always_allowed`, and `cues.watchers.always_allow.revoke`. Every Always On step
  is one `home.act {action, key, input}` the engine runs, answered `{summary, result}` or `changed`
  with its sentence alone; every summary action is one record (`id`, `label`, `needs`, `confirm`,
  `url`, `key`). Since the previous pin (`6a747236`): Always On runs on a Linux server the owner
  sets up, and Fermix creates, powers, replaces and deletes no server, so `home.provision.options`
  is gone with its schema and goldens; `home.act` takes `cancel`, `move_now`, `try_again`,
  `back_up_now`, `check_again`, `own_server`, `move_back` and `restore_here`, and its `input` only
  `recovery_passphrase`; the summary loses the state `choose`, every provider problem
  (`server_retired` among them), the needs `token` and `choice`, the notice `not_set_up`, Details'
  `providers`, `billing` and `next_location`, and the server's `type`, `location` and `ipv4`; `off`
  offers `own_server` as its action and `connect` none, with `own_server` and `cancel` beside it;
  Learn more opens the docs page with no anchor; Restore here is offered only where nothing answers
  from the server, asks the owner's word that the server is deleted or stopped for good, deletes
  nothing and runs as the job kind `home_restore` (`home_provision` gone); `home.enroll.open` and
  `home.enroll.withdraw` take no params, and `home.status`'s `enrollment` names no `kind`; the Cues
  screen's decision model offers OpenRouter, Venice, then OpenAI; and
  `computer_use.permissions.get`'s `display_server` names the session's display, its pixel route
  being the doctor's `pixel route` row.
- Protocol version: 2, daemon window 1..2.
- `CHECKSUMS.txt` holds the SHA-256 of each file, in `sha256sum` format.

Never edit these files by hand. To take a new engine contract, copy the six files again from a
committed engine, rewrite `CHECKSUMS.txt` with `sha256sum PROTOCOL.md protocol.schema.json
fixtures/requests.jsonl fixtures/success.jsonl fixtures/errors.jsonl fixtures/compatibility.jsonl`,
update the commit above, and run `cargo test -p fermix-client`: the golden fixtures then say whether
the app still decodes what the daemon sends.
