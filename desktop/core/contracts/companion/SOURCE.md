# Where this contract comes from

These four files are the engine's companion chat contract, copied byte for byte:

| file here | engine path |
|---|---|
| `PROTOCOL.md` | `apps/fermix_core/priv/companion/PROTOCOL.md` |
| `protocol.schema.json` | `apps/fermix_core/priv/companion/protocol.schema.json` |
| `fixtures/client_events.jsonl` | `apps/fermix_core/priv/companion/fixtures/client_events.jsonl` |
| `fixtures/server_events.jsonl` | `apps/fermix_core/priv/companion/fixtures/server_events.jsonl` |

- Repository: fermix (the engine)
- Commit: `7b59d3805782e2fab54b932105fb32f8ed33aebe` (dev), the tree they were copied from. The last
  commit to touch `priv/companion` is `178ed72551938a5e4a46c7dd429d88e1efb4e0a1` (2026-10-17,
  "feat(voice): Grok Voice as an opt-in voice engine beside GPT-Live (M70)").
- Protocol version: 3, daemon window 2..3: phone-side media (`media_fetch`, `media_path`,
  `media_missing`, `media_release`) and `row_updated`.
- `CHECKSUMS.txt` holds the SHA-256 of each file, in `sha256sum` format. The digests match the
  macOS app's pin (`Sources/FermixAppCore/Resources/Contracts/CHECKSUMS.txt`, `companion/` rows).

Never edit these files by hand. To take a new engine contract, copy the four files again, rewrite
`CHECKSUMS.txt` with `sha256sum PROTOCOL.md protocol.schema.json fixtures/*.jsonl`, update the
commit above, and run `cargo test --test companion_contract`: the golden fixtures then say whether
the app still decodes what the daemon sends.
