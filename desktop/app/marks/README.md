Vendor marks, copied byte for byte on 2026-09-25 from the macOS app's
`Apps/Fermix/Sources/FermixAppCore/Resources/VendorMarks/`, in the same layout: one directory per
kind (`providers/`, `channels/`, `plugins/`, `features/`, `meeting_platforms/`, `oauth_clients/`).
`PROVENANCE.json` holds the macOS record, word for word, for every mark this app draws; each record's
asset paths are relative to this directory, and its sha256 is the file's. A Linux-only fact about a
record is in its `linux_note` field, and the top-level `linux` object says what was left out and why.

`app/src/marks.rs` draws them and its tests check that its table, the records and these files agree.
Each record's `plate` says how a mark sits in its slot: `bleed` fills it and is clipped to its radius,
and `neutral` is drawn as is. The two single-ink marks (Ollama, xAI) are drawn in the label colour. A
key without a record gets its kind's neutral symbolic icon, never an invented mark. Never redraw,
trace or recolor these files.
