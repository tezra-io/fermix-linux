`permissions.jsonl` holds answers to `computer_use.permissions.get`, one per line, each
`{"name", "result"}`.

`computer_use_permissions_get_linux` is the engine's own Linux golden: GNOME on Wayland through the
desktop portal, with a stored screenshot "no", so `screen_capture` is false and `input_control` true.
Its `result` is copied byte for byte from the record of that name in
`apps/fermix_core/priv/management/fixtures/success.jsonl` of the engine (fermix) at commit
`c2bea7a01f76e77f08073ab33715b4ebba509b82`: the line is `{"name": "computer_use_permissions_get_linux",
"result": `, then the record's text after its `"result": ` without the response's and the record's
closing braces, then `}`. The vendored management contract is still the pinned engine's and lacks
that record. At the next pin bump, the contract's own golden replaces this line. Never edit it by
hand.

The other lines are hand-written in the engine's shape, for the sessions that golden does not show:
X11; X11 without XTEST; X11 whose X server the probe could not reach; GNOME on Wayland with both
halves; X11 where the desktop keeps a stored screenshot "no", which the X11 route never asks; KDE,
which has no pixel route and works by control; a session with no pixel route and no accessibility
bus; a pixel route the app does not know; rows and status words the app has never seen; a Linux
answer without rows; and a macOS answer. Each row's status and detail is one the engine's doctor rows
give for that session, except in `linux_unknown_rows`, whose `toolkit gtk3`, `input method` and
`window list` rows and `fail` status stand for rows and words no engine sends.

`linux_older_engine` is an engine from before the `pixel route` row (engine dev 5a864827 with compux
0.11.0) on X11: every other row as the current engine words it, and no `pixel route` row. A helper
older than 0.11 never meets an engine that sends rows: every such engine pins and installs 0.11 or
later at boot.
