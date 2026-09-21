# Fermix for Linux

Native GTK4 + libadwaita client, written in Rust, for the Fermix engine that systemd runs. The two ship
as one package: `fermix-desktop` carries the window, the engine and a private GTK4 runtime. The app
contains no engine of its own, writes no unit, reads no config file and holds no secret code:
everything it shows and changes goes over `daemon.sock` through the management protocol, and everything
that must work while the daemon is stopped goes through five typed operations of the packaged
`/usr/bin/fermix` CLI.

The design is `fermix/docs/design/MILESTONE_38_LINUX_COMPANION_APP.md` (M38), amended by
`docs/design/SINGLE_PACKAGE_AMENDMENT.md`, which is binding where the two differ. The rules for this
application are `docs/design/LINUX_DESIGN_SYSTEM_REDLINES.md`; the build order and slice acceptance are
`docs/design/IMPLEMENTATION_BRIEF.md`. Read them before touching a surface.

## Layout
```
App/Fermix/                # the crate: Cargo.toml, build.rs, src/, resources/, contracts/, tests/
packaging/                 # nfpm config, desktop entry, metainfo, D-Bus service, activation unit, icons
packaging/runtime/         # the private GTK4 runtime: RUNTIME.lock.json, build_runtime.sh, its manifest
packaging/docker/          # Dockerfile.runtime, Dockerfile.build, Dockerfile.smoke (one per matrix row)
engine/PIN.json            # the engine release this package carries, schema 2
scripts/                   # gates, each with a *_test.sh beside it
docs/design/               # redlines, brief, the single package amendment, reviewed captures
.github/workflows/         # app.yml, contract.yml (every push), packages.yml, runtime.yml,
                           # engine-release.yml (dispatch to tag), release-fermix-desktop.yml (tags),
                           # runtime-watch.yml (weekly)
```

## The contract with the engine
- `App/Fermix/contracts/management/` is a byte-identical copy of the engine's
  `apps/fermix_core/priv/management/`, pinned by `CHECKSUMS.txt` and `SOURCE.json`.
  `scripts/verify_contract.sh` checks the pin; `--source <fermix-checkout>` byte-compares upstream.
  Never edit a vendored file by hand; re-vendor from a committed engine commit.
- The app decodes the daemon's answers and renders its sentences. It never derives a state the daemon
  publishes, never authors a descriptor, and routes on published action ids, never on verb words.
- The version window comes from the schema's `x-supported-version-range`; per-method minimums gate a
  call before it is sent. A field a v2 daemon adds to a v1 method binds optionally with a declared absent
  rendering.
- `engine/PIN.json` names the engine release this package carries, by tag, source commit, certificate
  identity and one signed archive per target. It is written by the release bot from the engine's own
  dispatch and verified before it becomes a commit; no person edits it. An unpinned pin is not
  buildable, because the package contains the engine rather than depending on it.

## Working rules
- The daemon decides and the interface renders. No status vocabulary, refusal sentence or remediation
  title as a literal in `src/`.
- No colour literal outside `resources/icons/` and `resources/brand/`; no font size or family in CSS; no
  container the app draws itself.
- One `SettingsModel` instance, one `AdwNavigationSplitView` per window, one action map, one copy
  catalogue with a casing class per key, one metrics module, one motion module.
- GTK mutation on the main context only; GIO async for the socket and subprocesses; every exchange has a
  deadline; every socket and child is closed on every path; results from an older connection epoch never
  touch the model.
- Tests never touch the host: no real `daemon.sock`, no real `fermix`, no `~/.config`, no keyring.
  Socket tests use a fake peer; CLI tests use the fake `fermix` under `tests/fixtures/cli/`.
- Copy: sentence case source with a casing class; no em dashes, no exclamation marks, no version
  numbers, no wire tokens in a user-facing string.
- Vendor marks ship from `resources/VendorMarks/` byte-for-byte with their provenance; nothing redrawn.
- The window carries its own toolkit at `/usr/lib/fermix-desktop`, resolved by `RUNPATH`
  `$ORIGIN/../lib`. Exactly one environment variable is set, `GSETTINGS_SCHEMA_DIR`, by the application
  itself before GTK is initialised; every child launch restores the entry environment through
  `RuntimeEnv`. No launcher script, no `LD_LIBRARY_PATH`, no `XDG_DATA_DIRS` mutation.
- Gates before done: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`,
  `scripts/verify_contract.sh`, `scripts/check_app_identity.sh`, `scripts/check_vendor_marks.sh`,
  `scripts/check_copy.sh`, `scripts/check_no_network.sh`, and the container build
  (`scripts/container_build.sh`). A change under `packaging/` or `scripts/` also needs
  `scripts/build_packages.sh` and one `scripts/install_smoke.sh` row. Say which are not applicable and
  why.
- No AI attribution anywhere. Never commit or push unless the owner says so.
