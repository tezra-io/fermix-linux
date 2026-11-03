# Where this contract comes from

These four files are the engine's companion chat contract, copied byte for byte:

| file here | engine path |
|---|---|
| `PROTOCOL.md` | `apps/fermix_core/priv/companion/PROTOCOL.md` |
| `protocol.schema.json` | `apps/fermix_core/priv/companion/protocol.schema.json` |
| `fixtures/client_events.jsonl` | `apps/fermix_core/priv/companion/fixtures/client_events.jsonl` |
| `fixtures/server_events.jsonl` | `apps/fermix_core/priv/companion/fixtures/server_events.jsonl` |

- Repository: fermix (the engine)
- Commit: `b9dce743b4f31dfcfd27874d9f4baf43d6a096d3`, the engine v0.17.0. The last commit to touch
  `priv/companion` is `e0be70a6a0d5c79a46e7f69a4c19fd2fe24df96b` (2026-10-23, "feat(cues): cues
  and watchers, managed from the app, with proposals answered in its chat (M60)").
- Protocol version: 4, daemon window 3..4. Version 4 adds a mandate's proposals: the card
  (`proposal`), its end (`proposal_resolved`) and the owner's answer (`proposal_answer`), sent and
  taken only on version 4 connections the daemon places as the owner's. A version 2 client is
  refused.
- `CHECKSUMS.txt` holds the SHA-256 of each file, in `sha256sum` format. The digests match the
  macOS app 0.7.0's pin (`Sources/FermixAppCore/Resources/Contracts/CHECKSUMS.txt`, `companion/`
  rows).

Never edit these files by hand. To take a new engine contract, copy the four files again from a
committed engine, rewrite `CHECKSUMS.txt` with `sha256sum PROTOCOL.md protocol.schema.json
fixtures/*.jsonl`, update the commit above, and run `cargo test --test companion_contract`: the
golden fixtures then say whether the app still decodes what the daemon sends.
