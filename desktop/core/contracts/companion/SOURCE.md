# Where this contract comes from

These four files are the engine's companion chat contract, copied byte for byte:

| file here | engine path |
|---|---|
| `PROTOCOL.md` | `apps/fermix_core/priv/companion/PROTOCOL.md` |
| `protocol.schema.json` | `apps/fermix_core/priv/companion/protocol.schema.json` |
| `fixtures/client_events.jsonl` | `apps/fermix_core/priv/companion/fixtures/client_events.jsonl` |
| `fixtures/server_events.jsonl` | `apps/fermix_core/priv/companion/fixtures/server_events.jsonl` |

- Repository: fermix (the engine)
- Commit: `35a2c5ed588b1c398029c69358983891d1210076`, on the engine's dev branch, not a release. It
  is the last commit to touch `priv/companion` there (2026-11-02, "a reply's text_done carries
  ts").
- Protocol version: 7, daemon window 6..7. A client below 6 is refused.
  - Version 5: a watcher's follow-up, a `proposal` whose card names `follow_up` in place of a
    mandate's call, and the `always_allow` answer.
  - Version 6: what a turn is doing (`reasoning`, `tool_event.activity`), the owner's lists
    (`checklists_pull`, `checklist_pull`, `checklist_edit`, `checklists`, `checklist`) and the
    Always On notice (`home_status_changed`).
  - Version 7: files the owner attaches (`attach_file`, `attach_status`, `msg.attach_ids`, and
    `server_hello.attachments`).
- `CHECKSUMS.txt` holds the SHA-256 of each file, in `sha256sum` format.

`check_contracts.py` refuses a release until the engine pin moves to a release that serves 7 and
all three contracts are copied again from its tag: until then this commit is not the pinned
engine's.

Never edit these files by hand. To take a new engine contract, copy the four files again from a
committed engine, rewrite `CHECKSUMS.txt` with `sha256sum PROTOCOL.md protocol.schema.json
fixtures/*.jsonl`, update the commit above, and run `cargo test --test companion_contract`: the
golden fixtures then say whether the app still decodes what the daemon sends.
