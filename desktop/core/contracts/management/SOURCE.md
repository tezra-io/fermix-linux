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
- Commit: `9389f738a9a02dc12422fc04dc2b4fe3100b3bc9`, the engine v0.18.0 release tag. The last
  commit to touch `priv/management` is `6c85595cc094d5d584ec0b777043c8f503a20c3b` (2026-11-03,
  "feat(cues): the decision model's providers go OpenRouter, Venice, OpenAI everywhere"). Since
  v0.17.0, all within version 2, no method's minimum version moved: the `home.*` methods of Always
  On (a release build of the engine lists none of `home.status`, `home.act` and
  `home.provision.options` in `hello`, and refuses them), `cues.watchers.always_allow.revoke` and a
  watcher's `follow_up`, the X accounts feed preset, the source form's `choices` and `requirement`,
  an add with no name or a GitHub secret Fermix makes, `computer_use.permissions.get`'s `platform`,
  `display_server` and `rows`, a paired device's `kind` in `mobile.devices.list` (`phone`, or
  `desktop` for one an Always On move paired), and the plugin status `reinstall_failed`.
- Protocol version: 2, daemon window 1..2.
- `CHECKSUMS.txt` holds the SHA-256 of each file, in `sha256sum` format.

Never edit these files by hand. To take a new engine contract, copy the six files again from a
committed engine, rewrite `CHECKSUMS.txt` with `sha256sum PROTOCOL.md protocol.schema.json
fixtures/requests.jsonl fixtures/success.jsonl fixtures/errors.jsonl fixtures/compatibility.jsonl`,
update the commit above, and run `cargo test -p fermix-client`: the golden fixtures then say whether
the app still decodes what the daemon sends.
