# Fermix for Linux

The Fermix desktop app for Linux: a GTK4 and libadwaita window, written in Rust
and shipped as a Flatpak.

Fermix itself is a background service. The `fermix` package installs it and
your own systemd user manager runs it as `fermix.service`. This app is a client
of that service only: it talks to it over the sockets in `~/.fermix` and holds
no configuration and no secrets of its own. From it you set Fermix up, chat with
it, talk to it, and keep an eye on it.

Everything is in [`desktop/`](desktop/README.md): how to build, test, run and
install it.

```
desktop/     the app: core logic, the window, the Flatpak recipe, the design record
```
