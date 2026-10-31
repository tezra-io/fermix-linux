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
- Commit: `1d69c35ee2d77e307cb32fb79fd6a0d01d947ec9` (2026-10-31, branch `m75/hz-wave3`, not yet on
  `dev`), the M75 lean v1 Always On build. The last commit to touch `priv/management` is
  `d108d817ec8fa98eee95d4892340177f9c98647f` (2026-10-31, "docs(management): PROTOCOL.md lists
  the six home.provision methods in its method catalog beside every other method, so the forward
  deadlines table under After a move holds only the two long deadlines"). Since v0.17.0
  (`b9dce743`), all within version 2: the fifteen `home.*` methods (the home's status with its
  Always On `summary`, `disk` and `version`, recovery snapshots, enrollment, candidates, the move,
  and the six `home.provision.*` methods of the Hetzner provisioner), the `snapshots` section
  under pane `always_on`, `settings.sections.primary_missing`, a paired device's `kind`, the
  codes `primary_unreachable`, `device_class_method` and `unconfirmed`, the plugin status
  `reinstall_failed`, the job kind `home_provision`, and a cue watcher's `follow_up`.
- Protocol version: 2, daemon window 1..2.
- `CHECKSUMS.txt` holds the SHA-256 of each file, in `sha256sum` format.

Never edit these files by hand. To take a new engine contract, copy the six files again from a
committed engine, rewrite `CHECKSUMS.txt` with `sha256sum PROTOCOL.md protocol.schema.json
fixtures/requests.jsonl fixtures/success.jsonl fixtures/errors.jsonl fixtures/compatibility.jsonl`,
update the commit above, and run `cargo test -p fermix-client`: the golden fixtures then say whether
the app still decodes what the daemon sends.
