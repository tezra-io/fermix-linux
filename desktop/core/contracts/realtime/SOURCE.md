# Where this contract comes from

These four files are the engine's realtime voice contract, copied byte for byte:

| file here | engine path |
|---|---|
| `PROTOCOL.md` | `apps/fermix_core/priv/realtime/PROTOCOL.md` |
| `protocol.schema.json` | `apps/fermix_core/priv/realtime/protocol.schema.json` |
| `fixtures/client_events.jsonl` | `apps/fermix_core/priv/realtime/fixtures/client_events.jsonl` |
| `fixtures/server_events.jsonl` | `apps/fermix_core/priv/realtime/fixtures/server_events.jsonl` |

- Repository: fermix (the engine)
- Commit: `178ed72551938a5e4a46c7dd429d88e1efb4e0a1` (2026-10-17, "feat(voice): Grok Voice as an
  opt-in voice engine beside GPT-Live (M70)"), the last commit to touch `priv/realtime`
- Protocol version: 3, daemon window 1..3: Grok's `audio_delta.playback_id` and the app's
  `playback_progress`.
- `CHECKSUMS.txt` holds the SHA-256 of each file, in `sha256sum` format. The digests match the
  macOS app's pin at engine f2bd8bb0 (`Resources/Contracts/CHECKSUMS.txt`, `realtime/` rows).

Never edit these files by hand. To take a new engine contract, copy the four files again, rewrite
`CHECKSUMS.txt` with `sha256sum PROTOCOL.md protocol.schema.json fixtures/*.jsonl`, update the
commit above, and run `cargo test --test realtime_contract`: the golden fixtures then say whether
the app still decodes what the daemon sends.
