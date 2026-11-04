`permissions.jsonl` holds answers to `computer_use.permissions.get`, one per line, each
`{"name", "result"}`.

The engine's own Linux golden is `computer_use_permissions_get_linux` in the vendored management
contract (`core/contracts/management/fixtures/success.jsonl`): GNOME on Wayland through the desktop
portal, with a stored screenshot "no", so `screen_capture` is false and `input_control` true. The
tests read it there.

The lines here are hand-written in its shape, for the sessions that golden does not show:
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
