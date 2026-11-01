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
- Commit: `e1b7cbbb4808b174282fc9f357b077606cdc8b3e` (2026-11-01, branch `dev`), whose last change
  to `priv/management` is `6f829dba` ("fix(home): a move undone with the server Fermix created still
  held offers Delete your server too, through the one cleanup, once the undo is over"). Since
  v0.17.0 (`b9dce743`), all within version 2: the `home.*` methods (the home's status with its
  Always On `summary`, `disk` and `version`, recovery snapshots, enrollment, candidates, the move,
  `home.provision.options` and `home.act`), the `snapshots` section under pane `always_on`,
  `settings.sections.primary_missing`, a paired device's `kind`, the codes `primary_unreachable`,
  `device_class_method`, `unconfirmed` and `changed`, the plugin status `reinstall_failed`, the job
  kind `home_provision`, a cue watcher's `follow_up` with `always_allowed`, and
  `cues.watchers.always_allow.revoke`. Since the previous pin (`bcae16b9`): every Always On step is
  one `home.act {action, key, input}` the engine runs, answered `{summary, result}` or `changed`
  with its sentence alone; every summary action is one record (`id`, `label`, `needs`, `confirm`,
  `url`, `key`), `set_up` gone for `paste_token`; the summary's `key`, `notice`, `detail.providers`,
  `detail.lines` and `detail.progress`, with `detail.connect` and `detail.steps` gone; the problem
  `power_failed`; `home.status {after, wait_ms}`; `home.move.readiness`, `home.move.start`,
  `home.enroll.open`, `home.enroll.withdraw` and `home.snapshots.run` rail-only (refused on this
  socket by name); `home.provision.providers`, `connect`, `create`, `power`, `resources` and
  `delete` gone; the move checks the recovery passphrase against the one set, or sets it; the
  review's `service` item; `setup.session.create` an owner decision; the owner-only sentences naming
  the Fermix app, the setup page and the owner's own terminal; `job.cancel` answering once the run
  stopped; the Cues screen's decision model (`cues_decision_provider`); `home.provision.options`
  taking no params, answering for the provider in use (`provider {id, name}`), each offer with the
  engine's `label` and `price`; `create_server` and `replace_server` requiring `input.type` and
  `input.location`, the row the owner chose, and `start_again` needing `choice` where no size and
  place are kept; the summary's `outcome`; Delete your server beside a waiting server (`ready`, its
  blocked review, `undone`), Replace the server again after a replace that stopped, Start again
  after an account limit or no stock, and `token_replaced`; `nothing_in_stock` gone; and a release
  build serving no Always On (`hello` lists none of its methods, each refused `unavailable`
  {`capability`: `always_on`}).
- Protocol version: 2, daemon window 1..2.
- `CHECKSUMS.txt` holds the SHA-256 of each file, in `sha256sum` format.

Never edit these files by hand. To take a new engine contract, copy the six files again from a
committed engine, rewrite `CHECKSUMS.txt` with `sha256sum PROTOCOL.md protocol.schema.json
fixtures/requests.jsonl fixtures/success.jsonl fixtures/errors.jsonl fixtures/compatibility.jsonl`,
update the commit above, and run `cargo test -p fermix-client`: the golden fixtures then say whether
the app still decodes what the daemon sends.
