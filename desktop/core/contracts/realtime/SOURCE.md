# Where this contract comes from

These four files are the engine's realtime voice contract, copied byte for byte:

| file here | engine path |
|---|---|
| `PROTOCOL.md` | `apps/fermix_core/priv/realtime/PROTOCOL.md` |
| `protocol.schema.json` | `apps/fermix_core/priv/realtime/protocol.schema.json` |
| `fixtures/client_events.jsonl` | `apps/fermix_core/priv/realtime/fixtures/client_events.jsonl` |
| `fixtures/server_events.jsonl` | `apps/fermix_core/priv/realtime/fixtures/server_events.jsonl` |

- Repository: fermix (the engine)
- Commit: `eab5152f209480ffb2351e21473e762da8180623` (2026-09-13, "feat(voice): live voice engine
  slice, as it stood in the concurrent session"), the last commit to touch `priv/realtime`
- Protocol version: 2, daemon window 1..2. The installed fermix 0.11.0 ships the same bytes.
- `CHECKSUMS.txt` holds the SHA-256 of each file, in `sha256sum` format. The digests match the
  macOS app's pin (`Resources/Contracts/CHECKSUMS.txt`, `realtime/` rows).

Never edit these files by hand. To take a new engine contract, copy the four files again, rewrite
`CHECKSUMS.txt` with `sha256sum PROTOCOL.md protocol.schema.json fixtures/*.jsonl`, update the
commit above, and run `cargo test --test realtime_contract`: the golden fixtures then say whether
the app still decodes what the daemon sends.
