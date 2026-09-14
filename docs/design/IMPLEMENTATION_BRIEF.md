# Implementation brief, Fermix for Linux v1

**Updated:** 2026-09-13
**Binding design:** `fermix/docs/design/MILESTONE_38_LINUX_COMPANION_APP.md` (M38)
**Binding rules:** `docs/design/LINUX_DESIGN_SYSTEM_REDLINES.md` (redlines)

This brief is the work order. It names the crate stack, the module decomposition, the slices in build
order, and what each slice must prove before the next starts. An implementer reads M38 §5, §6.8 and §12.2
for the surface it is building, the redlines for the rules, and this file for the shape of the code.

## 1. Stack

| Concern | Choice | Reason |
|---|---|---|
| Language, toolkit | Rust 2021, `gtk4 = "0.11"` with feature `v4_16`, `libadwaita = "0.9"` with feature `v1_6`, `gio`/`glib` re-exported from gtk4 | M38 §5.1 floor GTK 4.16 / libadwaita 1.6. No feature above the floor is enabled; a newer symbol is a linker error on the floor host, which is the point |
| JSON | `serde`, `serde_json` | Typed structs per method result; the contract test decodes every vendored fixture through them |
| Strings | `gettext-rs` with `textdomain` `fermix-desktop`; every user-facing string goes through `copy::text(Key)` which marks it for extraction | M38 §6.7: one `.pot` per release, English only, casing applied to the English source |
| Async | GLib main context (`glib::spawn_future_local`), `gio::SocketClient::connect_async`, `gio::Subprocess` | M38 §12.2: no second async runtime |
| Resources | `glib-build-tools` in `build.rs` compiles `resources/fermix.gresource.xml` (icons, brand, vendor marks, one small CSS file) | One bundle, no runtime file lookups |
| Tests | `cargo test`; widget tests behind `FERMIX_GTK_TESTS=1` under `xvfb-run` in the container | A test that needs a display never runs by accident on a headless host |
| Packaging | nFPM from one `packaging/nfpm-fermix-desktop.yaml`; the build runs inside `packaging/docker/Dockerfile.build` (`debian:trixie` plus the Rust toolchain, GTK and libadwaita dev packages, nfpm, appstream, desktop-file-utils, xvfb) | The same container builds locally and in CI; GitHub's runners carry a GTK below the floor |

The crate is `App/Fermix/`, binary `fermix-desktop`. A second binary, `fixture-daemon`, serves the vendored
fixtures over a Unix socket for development, captures and tests. `tests/fixtures/cli/fermix` is a shell
script standing in for `/usr/bin/fermix`, answering each typed operation from a JSON file chosen by
`FERMIX_FAKE_CLI_STATE`.

The path of the packaged CLI is a compile-time constant, `FERMIX_CLI_PATH`, default `/usr/bin/fermix`,
overridden by the environment variable `FERMIX_DESKTOP_CLI` **in debug builds only**. Likewise
`FERMIX_DESKTOP_FIXTURE_HOME` (debug only) points the app at a fixture daemon's socket and fake CLI. These
are development configurations, declared and gated by `cfg(debug_assertions)`, never a runtime branch in a
release build.

## 2. Module decomposition

```
src/
  main.rs                  parse argv (--capture DIR in debug), build App, run
  app.rs                   FermixApp: GtkApplication subclass, single instance, action map + accels, menus
  window.rs                FermixWindow: AdwApplicationWindow, split view, sidebar, presentations, geometry
  copy.rs                  the catalogue: Key enum, text(Key), casing class, header-capitalization
  metrics.rs               the spacing scale and layout budgets
  motion.rs                durations, reduced-motion switch
  registry.rs              SurfaceRegistry: hand-built surfaces with counterpart or exemption sentence
  management/
    framing.rs             packet-4 read/write with the 4 MiB ceiling, frame validation before allocation
    transport.rs           gio async Unix socket exchange, one deadline, close on every path
    client.rs              ManagementClient: hello negotiation, request/response, typed errors, epoch
    contract.rs            reads the vendored schema at compile time: version window, method minimums
    types.rs               serde structs for every result the app decodes
    errors.rs              ManagementError: wire error codes + details.sentence, transport failures
  service/
    runner.rs              ServiceRunner: the five typed CLI operations over gio::Subprocess
    types.rs               the JSON envelope and result structs (service status, install, restart, export)
  session/
    desktop.rs             DesktopSession: display/session facts, browser launch, file manager, notifications
    autostart.rs           the ~/.config/autostart entry writer
    state.rs               $XDG_STATE_HOME persistence (geometry, sidebar, last pane)
  models/
    settings_model.rs      the one SettingsModel
    home.rs doctor.rs logs.rs onboarding.rs recovery.rs
    jobs.rs                JobRunner
    ledger.rs              PermissionLedger
  ui/
    home.rs doctor.rs logs.rs recovery.rs about.rs shortcuts.rs
    settings/
      mod.rs               the Settings presentation: pane list, detail host, Restart action, banner
      pane_list.rs         four groups, thirteen panes, search
      descriptor_form.rs   one section's rows over AdwPreferencesGroup
      descriptor_row.rs    one control per row kind, commit/revert semantics
      providers.rs channels.rs integrations.rs meetings.rs computer.rs permissions.rs voice.rs
      dialogs/             secret.rs, sign_in.rs, channel.rs, consent.rs, oauth_client.rs, workspace.rs,
                           model_picker.rs, restart.rs
    onboarding/
      mod.rs welcome.rs starting.rs connect_ai.rs about_you.rs applying.rs ready.rs boot_failed.rs
    widgets/
      mark.rs              the 36 px leading artwork slot
      status_pill.rs       the text pill
      checklist_row.rs     the progress row for Starting/Applying
  bin/
    fixture_daemon.rs
resources/
  fermix.gresource.xml
  style.css                variables and classes only; no colours, no font sizes
  icons/                   io.tezra.Fermix.svg, io.tezra.Fermix-symbolic.svg, hicolor rasters
  brand/                   mascot, wordmark
  VendorMarks/             PROVENANCE.json, ROSTER.json, providers/, channels/, plugins/, oauth_clients/,
                           meeting_platforms/, features/
contracts/
  management/              vendored engine export
  SOURCE.json CHECKSUMS.txt
tests/
  contract.rs copy.rs structure.rs management_client.rs service_runner.rs widgets.rs
  fixtures/cli/            fake fermix and its state files
```

One owner per concept. `SettingsModel` is constructed once in `app.rs` and handed to every view that reads
it; a test asserts the count. Views own no state the model publishes.

## 3. Invariants every slice keeps

1. `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` green on macOS
   (Homebrew GTK) and inside the container. Warnings are errors.
2. The structure gates in `tests/structure.rs` pass: no colour literal, no font size or family in CSS,
   no status/refusal/remediation literal in `src/`, exactly one `AdwNavigationSplitView` construction,
   one `SettingsModel::new`, every `copy::Key` used at least once and every user-facing `&str` in `ui/`
   coming from `copy::text`.
3. `scripts/verify_contract.sh`, `scripts/check_app_identity.sh`, `scripts/check_vendor_marks.sh`,
   `scripts/check_copy.sh` pass; each ships with its `_test.sh`.
4. The fixture daemon and fake CLI drive every surface; a surface that cannot be shown from fixtures has
   no fixture, which is a defect in the slice, not in the fixtures.
5. Nothing in the tree contacts the host: no `~/.fermix`, no `/usr/bin/fermix`, no `systemctl`, no
   keyring, in any test.

## 4. Slices

### LA1, foundation

Delivers: the crate, `build.rs`, resources bundle, `copy.rs` with the full catalogue keys used by every
later slice (text may be refined later but keys are named now), `metrics.rs`, `motion.rs`,
`management/*`, `service/*`, `session/*`, `app.rs`, `window.rs` with the sidebar, breakpoint, pinned
Settings row, action map, accelerators, primary menu, shortcuts dialog, About dialog, geometry
persistence; placeholder `AdwStatusPage`s for Home, Doctor, Logs, Settings; the fixture daemon and the
fake CLI; the vendored contract with `SOURCE.json` and `CHECKSUMS.txt`; `scripts/verify_contract.sh`,
`scripts/check_app_identity.sh`, `scripts/check_copy.sh`, `scripts/container_build.sh` and their tests;
`packaging/docker/Dockerfile.build`; `.github/workflows/app.yml` and `contract.yml`.

Proves:
- `tests/contract.rs`: every checksum matches; every record in `fixtures/success.jsonl` decodes into
  its typed result; the `compatibility.jsonl` golden with every optional plugin-row field absent decodes;
  an N-1 `overview.get` without `restart_reasons` and a doctor result without `remediation` decode and
  render their declared absent forms; the version window is read from the schema and no second key
  exists.
- `tests/management_client.rs` against a fake peer: a valid exchange; a peer that accepts and never
  answers (deadline fires, socket closed); a frame over 4 MiB (refused before allocation); a short
  frame; a response with a different `request_id` (discarded); a result from an older epoch (dropped).
- `tests/service_runner.rs` against the fake CLI: each of the five operations parses its envelope; a
  non-zero exit with an error envelope becomes a typed error carrying the sentence; stdout beyond the
  cap is refused; a hung child is cancelled and reaped.
- The window opens from the fixture home, the sidebar is keyboard-traversable, `Ctrl+comma` enters
  Settings and Escape leaves it, the shortcuts dialog lists every accelerator from the action map.
- The container builds the crate and runs the tests under `xvfb-run`.

### LA2, models and the three pages, Settings shell, descriptor form

Delivers: `SettingsModel`, `HomeModel`, `DoctorModel`, `LogsModel`, `RecoveryModel`, `JobRunner`;
Home, Doctor, Logs, Recovery views; the Settings presentation with the searchable pane list, the
`DescriptorForm` and `DescriptorRow` for all six kinds, the external-change banner, the Restart action
and its dialog (Restart Now, Restart When Idle shown disabled with the daemon's reason when the CLI
refuses idle mode, Cancel); the debug capture mode `--capture DIR` that renders each reference state
to PNG through `gtk::WidgetPaintable`.

Proves:
- Home renders the four reference states from fixtures (running without attention, one actionable
  failure, setup incomplete, daemon unavailable); the two switches call the fake CLI and revert with the
  sentence on refusal; Runtime details is collapsed by default and its facts are the daemon's.
- Doctor renders healthy, failed-with-evidence, running and timed-out; a failed row's button routes by
  remediation kind; network checks run only from the explicit action.
- Logs polls only while visible and unpaused, keeps scroll position across polls, resets on
  `cursor_expired` with the sentence, and shows the file/journal caption.
- Descriptor rows: toggle and choice commit on selection; text and number commit on Enter and on
  focus loss; Escape restores the daemon value and sends nothing; a refusal reverts and shows
  `details.sentence` under the row; the optimistic value holds until the accepted re-read; a refresh
  during an edit does not disturb the field; unsent list additions survive navigation.
- Settings: pinned row, menu and `Ctrl+comma` reach the same presentation; typing in the pane list
  filters it; Back and Escape restore the previous surface, scroll and focus; the selected pane survives
  leaving and returning; one Restart action across pane changes; the banner takes priority.
- `JobRunner`: dismissing a job view detaches polling; reopening finds it through `job.list`; explicit
  Cancel calls `job.cancel`; polling never exceeds the job's `budget_ms` or 2,000 polls.
- Captures for every state above land in `docs/design/captures/` with `INDEX.md` rows (decision column
  left `pending`).

### LA3, the hand-built panes and dialogs

Delivers: Providers (rows with marks and inline verbs, sub-page with auth mode, key secret row, base
URL, Sign Out, Use as Primary with confirmation, model picker dialog, probe), Channels (switch rows and
descriptor sub-pages, secrets first), Integrations (flat filtered searchable list, install→enable on the
switch, consent dialog, detail with status sentence, next step and verbs routed by action id, settings
rows, OAuth client dialog, workspace dialog), Meetings (switch-headed install job, Google Meet and Zoom
sections with marks, sign-in state from `setup.detect`), Computer (switch header, install job, probe rows,
Refresh, the arm64 / no display / Wayland statements), Permissions (the §7.4 ledger), Voice (the honesty
statement above capture controls plus descriptor rows), the secret dialog (the one place a secret entry
exists), `PermissionLedger`, `SurfaceRegistry` entries with exemptions, vendor marks and `check_vendor_marks.sh`.

Proves:
- Every state in M38 §6.8's Integrations and Meetings rows renders from fixtures; verbs route on ids;
  a refused stage stops the chain with the sentence; the detail re-reads the live row.
- Providers: a row leads with a verb the daemon answers (`auth_modes` alone is not the signal).
- Secrets: blank is never sent; the entry exists in one file; cleared on close.
- The registry lists every hand-built surface with a counterpart or an exemption sentence; the
  exemptions are exactly: the SMAppService row, `computer_use.grant.start`, computer history, the Choose
  apps picker.
- Captures for the Providers, Integrations, Meetings and dialog reference states.

### LA4, onboarding and lifecycle

Delivers: the Setup assistant (Welcome, Starting, Connect Your AI, About You, Applying, Ready, Boot
Failed) inside the window with the stable bottom bar; preflight refusals from `service status`;
activation through `service install --json` with the checklist driven by the CLI's phases, `hello`, a
loopback `GET /health/live` over `gio::SocketClient`, and `setup.state.get`; the 90-second ceiling and
the five named failure states; browser sign-in through `gtk::UriLauncher` with the URL shown as copyable
text; `auth.import.start`; the personalization write; conditional restart through `restart --json`;
Ready's next-step rows; engine skew (typed alignment from `service status`) and GUI self-skew (compiled
build id versus `/usr/share/fermix-desktop/build.json`) as Home attention rows and the Finish Updating
dialog; the autostart writer; GNotification for attention changes while the app runs.

Proves:
- Every assistant screen and failure state renders from fixtures; Enter advances, Escape goes back;
  Starting's only way out is Cancel; a refused About You write stays on About You with the sentence; a
  home that needs no restart never shows the restart row; closing the assistant never marks completion.
- Linger denied and loginctl absent render M38 §6.5's copy with the toggle off.
- Skew: aligned, pending restart, unknown id, conflict, stale GUI each render their state.
- Captures for every assistant state.

### LP1, packaging and the release rail

Delivers: `packaging/io.tezra.Fermix.desktop`, `io.tezra.Fermix.metainfo.xml`, `dbus/io.tezra.Fermix.service`,
`systemd/app-io.tezra.Fermix.service`, `icons/hicolor/` (authored SVG, symbolic SVG, rasters 16 to 256),
`build.json` manifest, `nfpm-fermix-desktop.yaml` with the declared toolkit relations and the exact
`fermix` version relation; `scripts/build_packages.sh` (runs in the container: build, gates, nfpm deb and
rpm, `dpkg-shlibdeps` and `rpm -qp --requires` coverage check); `scripts/fetch_engine.sh` and
`scripts/verify_engine.sh` over `engine/PIN.json` (download the engine's Linux packages from its release,
verify sha256 and cosign against the recorded identity); `scripts/install_smoke.sh` (a systemd-enabled
Debian container: install the engine package and the desktop package, `fermix service install --json`,
`fermix service status --json`, launch `fermix-desktop --capture` under `xvfb-run`);
`.github/workflows/release-fermix-desktop.yml` (tag `fermix-desktop-vX.Y.Z`, strict semver refusal,
matrix `ubuntu-24.04` and `ubuntu-24.04-arm` each building in the container, cosign sign-blob, the
smoke against the built assets, `gh release create --latest=false` with the desktop packages, and the
paired engine packages re-attached after verification so one release page carries everything an
installer needs); `README.md` install instructions; `docs/ACCEPTANCE_RUNBOOK.md` with M38 §13.2's
thirteen hand-verified gates.

Proves:
- `desktop-file-validate` and `appstreamcli validate` pass; `check_app_identity.sh` finds the one string
  in all six places.
- Both packages build for both architectures in the container; the install smoke passes on arm64
  locally (Docker) and on both architectures in CI.
- The release workflow refuses a tag with `-` or `:`, and a `PIN.json` that is half filled.

### Review passes

After LA2, LA3, LA4 and LP1 a reviewer runs every gate, opens the captures and checks them against the
redlines §3 (keyboard and clicks) and §6 (surfaces), and fixes what fails. Every review returns the list
of open items with the file and the sentence of the redline it violates.

### Reconciliation with the engine's typed CLI (added 2026-09-13)

The engine lane emits the typed CLI from `Fermix.CLI.MachineOutput` and `Fermix.CLI.Service.Status`
in the engine checkout beside this repository (`../fermix/apps/fermix_core/lib/fermix/cli/`). The review pass (LR) must align
this crate with what the engine actually prints, not with the field list in section 4:

- `service/types.rs` decodes every field `Status` publishes (`binding {state, home, reason}`, `unit
  {effective_path, vendor, legacy_generated, foreign, need_daemon_reload}`, `enabled`, `active`,
  `sub_state`, `pid`, `invocation_id`, `restart_count`, `linger`, `path_source`, `listener {port,
  origin, source}`, `installed {…, integrity}`, `running`, `alignment`) with the nullability the
  engine's tests show, and `ServiceError` carries every code in `MachineOutput.codes/0` with the
  engine's own sentence rendered verbatim.
- `tests/fixtures/cli/` states are regenerated from the engine's expected JSON (read
  `apps/fermix_core/test/fermix/cli/service/{packaged,status}_test.exs` and `machine_output_test.exs`),
  so the fake CLI cannot answer a shape the engine never prints.
- When `apps/fermix_core/priv/cli/` exists (the engine review pass creates it), vendor it into
  `App/Fermix/contracts/cli/`, extend `SOURCE.json`, `CHECKSUMS.txt` and `scripts/verify_contract.sh`
  to cover it, and make `tests/contract.rs` decode every CLI golden. Until it exists, record the
  gap in the handoff as the first follow-up.

## 5. What is deliberately not in v1

- No tray, no tray copy (M38 §5.2 makes it optional; keeping it out keeps the surface invariant trivially
  true).
- No `lifecycle.prepare_idle`; "Restart When Idle" is disabled with the CLI's sentence until the engine
  publishes protocol 3.
- No translations; the `.pot` ships, no `.po` does.
- No AppStream catalog publication and no apt/rpm repository; both packages ship as release assets
  until the repository service exists.
- No Wayland computer use; the Computer pane states the probe's result.
- No voice client, no pet.
