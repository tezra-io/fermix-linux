# Where this contract comes from

These four files are the engine's companion chat contract, copied byte for byte:

| file here | engine path |
|---|---|
| `PROTOCOL.md` | `apps/fermix_core/priv/companion/PROTOCOL.md` |
| `protocol.schema.json` | `apps/fermix_core/priv/companion/protocol.schema.json` |
| `fixtures/client_events.jsonl` | `apps/fermix_core/priv/companion/fixtures/client_events.jsonl` |
| `fixtures/server_events.jsonl` | `apps/fermix_core/priv/companion/fixtures/server_events.jsonl` |

- Repository: fermix (the engine)
- Commit: `95ee575b4de961bcc43f138038e06b8c379f4ba8`, the released engine v0.16.0, the tree they
  were copied from. The last commit to touch `priv/companion` is
  `83464183af6e998b28f0796f3e418238a1f704f3` (2026-10-18, "fix(voice): a picture or file Fermix
  sends reaches the Mac chat, from a GPT-Live or Grok call too"), which changed only the prose: a
  picture or file a call sends is an ordinary media row.
- Protocol version: 3, daemon window 2..3: phone-side media (`media_fetch`, `media_path`,
  `media_missing`, `media_release`) and `row_updated`.
- `CHECKSUMS.txt` holds the SHA-256 of each file, in `sha256sum` format. The digests match the
  macOS app 0.6.0's pin (`Sources/FermixAppCore/Resources/Contracts/CHECKSUMS.txt`, `companion/`
  rows).

Never edit these files by hand. To take a new engine contract, copy the four files again, rewrite
`CHECKSUMS.txt` with `sha256sum PROTOCOL.md protocol.schema.json fixtures/*.jsonl`, update the
commit above, and run `cargo test --test companion_contract`: the golden fixtures then say whether
the app still decodes what the daemon sends.
