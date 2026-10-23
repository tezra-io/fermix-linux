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
- Commit: `95ee575b4de961bcc43f138038e06b8c379f4ba8`, the released engine v0.16.0. The last commit
  to touch `priv/management` is `7b8eba6a439d31de9c877fc63887b2a2b39e3d8f` (2026-10-21, "fix(mcp):
  an owner's client ID and secret stay with the authorization server they were signed in with (M74
  E5)"): the `mcp.servers.*` methods, `auth.start mcp:<name>` and the optional
  `environment_override` on `secret.set` and `secret.clear`, all added within version 2
- Protocol version: 2, daemon window 1..2.
- `CHECKSUMS.txt` holds the SHA-256 of each file, in `sha256sum` format. The digests match the
  macOS app 0.6.0's pin (`Sources/FermixAppCore/Resources/Contracts/CHECKSUMS.txt`, `management/`
  rows).

Never edit these files by hand. To take a new engine contract, copy the six files again from a
committed engine, rewrite `CHECKSUMS.txt` with `sha256sum PROTOCOL.md protocol.schema.json
fixtures/requests.jsonl fixtures/success.jsonl fixtures/errors.jsonl fixtures/compatibility.jsonl`,
update the commit above, and run `cargo test -p fermix-client`: the golden fixtures then say whether
the app still decodes what the daemon sends.
