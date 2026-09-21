# Reference captures

Rendered by the application's own capture mode against the fixture daemon, at
the default window size and text scale, in both colour schemes. Every row
carries what a reviewer needs to read the image: which fixtures produced it,
which commit drew it, and which toolkit drew it.

Take them with `scripts/capture.sh --container`. The ones that get reviewed are
the container's: it is Linux, it carries the toolkit floor, and it draws the
icons a desktop draws. A capture taken on a development Mac is honest about
layout and copy and silent about icons, because that host has no icon theme at
all.

A capture that exists is not an accepted capture. The reviewer column says who
last looked at the image: `automated review` is a review pass that opened it and
read it against `LINUX_DESIGN_SYSTEM_REDLINES.md`, and `pending` is nobody. The
decision column is the owner's, and no review pass fills it in. Taking a capture
again makes a new image, so both columns go back to `pending` with it.

| File | Fixture | Commit | GTK | libadwaita | Runtime key | Window | Text scale | Scheme | Reviewer | Decision |
|---|---|---|---|---|---|---|---|---|---|---|
| `home_setup_required-light.png` | setup_required | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `home_setup_required-dark.png` | setup_required | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `home_attention-light.png` | restart_pending | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `restart_dialog-light.png` | restart_pending | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `home_attention-dark.png` | restart_pending | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `restart_dialog-dark.png` | restart_pending | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `home_not_running-light.png` | not_running | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `home_not_running-dark.png` | not_running | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_external_change-light.png` | external_change | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_external_change-dark.png` | external_change | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `recovery-light.png` | unreadable | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `recovery-dark.png` | unreadable | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `doctor_healthy-light.png` | doctor_healthy | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `doctor_healthy-dark.png` | doctor_healthy | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `doctor_failed-light.png` | doctor_failed | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `doctor_failed-dark.png` | doctor_failed | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `logs_empty-light.png` | logs_empty | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `logs_empty-dark.png` | logs_empty | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_integrations_installed-light.png` | integrations_states | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_integrations_available-light.png` | integrations_states | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_integrations_detail-light.png` | integrations_states | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `consent_dialog-light.png` | integrations_states | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_integrations_installed-dark.png` | integrations_states | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_integrations_available-dark.png` | integrations_states | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_integrations_detail-dark.png` | integrations_states | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `consent_dialog-dark.png` | integrations_states | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_meetings_signed_out-light.png` | meetings_signed_out | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_meetings_signed_out-dark.png` | meetings_signed_out | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_computer_not_installed-light.png` | computer_states | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_computer_not_installed-dark.png` | computer_states | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_computer_wayland-light.png` | computer_wayland | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_computer_wayland-dark.png` | computer_wayland | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `welcome-light.png` | onboarding_welcome | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `welcome-dark.png` | onboarding_welcome | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `starting_running-light.png` | onboarding_starting | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `starting_running-dark.png` | onboarding_starting | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `starting_failed_linger-light.png` | onboarding_linger_denied | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `starting_failed_linger-dark.png` | onboarding_linger_denied | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `boot_failed-light.png` | onboarding_boot_failed | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `boot_failed-dark.png` | onboarding_boot_failed | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `connect_ai-light.png` | onboarding_connect_ai | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `connect_ai_waiting-light.png` | onboarding_connect_ai | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `connect_ai-dark.png` | onboarding_connect_ai | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `connect_ai_waiting-dark.png` | onboarding_connect_ai | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `about_you-light.png` | onboarding_about_you | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `about_you-dark.png` | onboarding_about_you | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `about_you_refused-light.png` | onboarding_refused_personalization | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `about_you_refused-dark.png` | onboarding_refused_personalization | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `applying_no_restart-light.png` | onboarding_no_restart | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `applying_no_restart-dark.png` | onboarding_no_restart | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `applying_restart-light.png` | onboarding_restart_needed | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `applying_restart-dark.png` | onboarding_restart_needed | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `ready-light.png` | onboarding_ready | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `ready-dark.png` | onboarding_ready | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `skew_attention-light.png` | onboarding_skew | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `skew_attention-dark.png` | onboarding_skew | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_secret_store_file-light.png` | secret_store_file | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_secret_store_file-dark.png` | secret_store_file | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `home_running-light.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `doctor_running-light.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `logs_populated-light.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_memory-light.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_sandbox-light.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_providers-light.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_providers_detail-light.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_channels-light.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_meetings_signed_in-light.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_computer_installed-light.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_permissions-light.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_voice-light.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_model_picker-light.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `secret_dialog-light.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `settings_secret_store_keyring-light.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `secret_keyring_locked-light.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `secret_store_absent-light.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | light | pending | pending |
| `home_running-dark.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `doctor_running-dark.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `logs_populated-dark.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_memory-dark.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_sandbox-dark.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_providers-dark.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_providers_detail-dark.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_channels-dark.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_meetings_signed_in-dark.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_computer_installed-dark.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_permissions-dark.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_voice-dark.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_model_picker-dark.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `secret_dialog-dark.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `settings_secret_store_keyring-dark.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `secret_keyring_locked-dark.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
| `secret_store_absent-dark.png` | default | 21e938e | 4.16.7 | 1.6.9 | a89ab0607a323501 | 880x560 | 1.0 | dark | pending | pending |
