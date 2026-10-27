# Security Policy

## Reporting a vulnerability

**Please do not open a public issue for security vulnerabilities.**

Report privately by email to **[security@tezra.io](mailto:security@tezra.io)**.
The same address covers every Fermix repository.

Please include:

- a description of the issue and its impact,
- steps to reproduce (or a proof of concept),
- the affected version / commit, and
- any suggested remediation.

We aim to acknowledge reports within a few days and will coordinate a fix and
disclosure timeline with you. Never include real secrets (API keys, tokens) in
a report — redact them.

## Supported versions

Security fixes ship in the next `fermix-desktop` release. Always run the
latest release.

## Scope

Fermix for Linux is the native GTK4 companion for the Fermix engine, packaged
as `fermix-desktop` beside the `fermix` engine package.

For the Fermix engine's own boundary (secret handling, the sandbox, channel
ingress, signed upgrades), see the [Fermix security policy](https://github.com/tezra-io/fermix/blob/main/SECURITY.md).

A process already running as your user is outside Fermix for Linux's
boundary. It can change how Fermix starts, and ask your system for anything
Fermix can, behind the same password prompt. So an update's checks (the
signed feed, then the package's size and sha256 before each install pass)
protect against a tampered download or server, not against your own account:
a process running as you could rewrite the downloaded package after the last
check, before PackageKit or the install command the update dialog shows
opens it, but it could as well ask PackageKit to install a package of its
own.
