//! The copy catalogue.
//!
//! One row per key: the English sentence-case source and its casing class.
//! [`text`] renders a key by running its source through gettext for extraction
//! and then applying Header Capitalization to `Header` rows only.
//!
//! Three rules hold here and nowhere else in the tree:
//!
//! 1. Every word the interface shows is a key. A user-facing string literal
//!    anywhere else is a defect, and `tests/structure.rs` says so.
//! 2. No word the daemon owns is a key. Status sentences, refusal sentences,
//!    remediation titles, restart reasons and plugin words come off the wire.
//!    What the catalogue does own is the English rendering of the *closed
//!    vocabularies* the wire publishes as atoms, because an atom is not a word:
//!    the eight check statuses and the ten job phases have no display form on
//!    the wire and one has to exist somewhere.
//! 3. The casing transform runs over the English source only. A translated
//!    string carries its own language's capitalization, and Header
//!    Capitalization is an English rule (M38 section 6.7).

use gettextrs::gettext;

/// Which capitalization a row renders in.
///
/// GNOME wants Header Capitalization for headings, header-bar headings,
/// buttons and menu items, and sentence capitalization for body text, field
/// labels and switch labels. The column is what lets one deck serve both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Casing {
    /// Window and page titles, group headings, buttons, menu items and dialog
    /// titles. Rendered in Header Capitalization.
    Header,
    /// Body, field labels, switch labels, captions and statements. Rendered as
    /// written.
    Sentence,
}

/// One catalogue row.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub key: Key,
    pub source: &'static str,
    pub casing: Casing,
}

/// Every key the product needs.
///
/// The order of this enum is the order of [`CATALOGUE`], and a key is looked up
/// by its own discriminant, so the two can never disagree without
/// `tests/copy.rs` failing. [`Key::Count`] is not a key; it is how many there
/// are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    // ---- Identity -------------------------------------------------------
    ProductName,

    // ---- Page titles ----------------------------------------------------
    PageHome,
    PageDoctor,
    PageLogs,
    PageSettings,
    PageRecovery,
    PageSetup,

    // ---- Primary menu ---------------------------------------------------
    MenuSettings,
    MenuRunDoctor,
    MenuRestart,
    MenuKeyboardShortcuts,
    MenuAbout,
    MenuQuit,
    MenuPrimaryAccessible,

    // ---- Navigation -----------------------------------------------------
    BackToFermix,
    ToggleSidebarAccessible,
    SidebarAccessible,

    // ---- Shortcuts dialog -----------------------------------------------
    ShortcutsTitle,
    ShortcutsGroupGeneral,
    ShortcutsGroupNavigation,
    ShortcutsGroupActions,
    ShortcutOpenSettings,
    ShortcutShowHome,
    ShortcutShowDoctor,
    ShortcutShowLogs,
    ShortcutGoBack,
    ShortcutSearchSurface,
    ShortcutRunDoctor,
    ShortcutRestart,
    ShortcutToggleSidebar,
    ShortcutCloseWindow,
    ShortcutQuit,
    ShortcutAbout,
    ShortcutKeyboardShortcuts,

    // ---- About ----------------------------------------------------------
    AboutDeveloper,
    AboutComments,
    AboutWebsite,

    // ---- Shared controls ------------------------------------------------
    ActionCancel,
    ActionClose,
    ActionContinue,
    ActionBack,
    ActionTryAgain,
    ActionRefresh,
    ActionExport,
    ActionCopyCommand,
    ActionCopyCommands,
    ActionShowMeHow,
    ActionOpenSettingsPane,
    ActionSearchAccessible,
    ActionMoreAccessible,
    BusyQueueFull,
    StateOn,
    StateOff,
    StateNotReported,

    // ---- Home -----------------------------------------------------------
    HomeGroupBackground,
    HomeGroupAttention,
    HomeGroupRuntimeDetails,
    HomeRowStatus,
    HomeStatusRunning,
    HomeStatusSetupRequired,
    HomeStatusRestartToFinishUpdating,
    HomeStatusNotRunning,
    HomeSwitchRunInBackground,
    HomeSwitchOpenAtLogin,
    HomeAttentionEmpty,
    HomeRuntimeEngine,
    HomeRuntimeManagementProtocol,
    HomeRuntimeUptime,
    HomeRuntimeProvider,
    HomeRuntimeChannels,
    HomeRuntimeSkills,
    HomeRuntimeTools,
    HomeRuntimeService,
    HomeRuntimeSession,
    HomeRuntimeUnavailable,
    HomeRuntimeNone,
    HomeUptimeDaysHours,
    HomeUptimeHoursMinutes,
    HomeUptimeMinutes,
    HomeUptimeJustStarted,
    HomeToolbarContinueSetup,
    HomeToolbarFinishUpdating,

    // ---- Home attention rows, one per detail_key family -----------------
    AttentionProviderTitle,
    AttentionProviderAction,
    AttentionPersonalizationTitle,
    AttentionPersonalizationAction,
    AttentionChannelTitle,
    AttentionChannelAction,
    AttentionRealtimeTitle,
    AttentionRealtimeAction,
    AttentionRestartPendingTitle,
    AttentionLegacyServiceUnitTitle,
    AttentionEnginePathBaselineTitle,
    AttentionOpenDoctorAction,

    // ---- Restart --------------------------------------------------------
    RestartDialogTitle,
    RestartDialogBody,
    RestartConversationsKnown,
    RestartConversationsUnknown,
    RestartNow,
    RestartWhenIdle,
    FinishUpdatingDialogTitle,
    FinishUpdatingDialogBody,

    // ---- Banners --------------------------------------------------------
    BannerExternalChangeTitle,
    BannerReloadFromDisk,

    // ---- Settings, groups and panes -------------------------------------
    SettingsGroupAssistant,
    SettingsGroupConnections,
    SettingsGroupCapabilities,
    SettingsGroupSystem,
    PaneProviders,
    PanePersonality,
    PaneMemory,
    PaneChannels,
    PaneIntegrations,
    PaneVoice,
    PaneMeetings,
    PaneComputer,
    PaneCodingAgents,
    PaneSearch,
    PaneImages,
    PaneSandbox,
    PanePermissions,
    SettingsSearchAccessible,
    SettingsSearchEmpty,
    ListAdd,
    ListRemoveAccessible,

    // ---- Secrets --------------------------------------------------------
    SecretStored,
    SecretReplace,
    SecretRemove,
    SecretAdd,
    SecretDialogTitleAdd,
    SecretDialogTitleReplace,
    SecretFieldLabel,
    SecretStoreRowLabel,
    SecretDialogConfirm,

    // ---- Providers ------------------------------------------------------
    ProvidersGroupPrimary,
    ProvidersGroupProviders,
    ProviderChooseModel,
    ProviderModelDialogTitle,
    ProviderSignIn,
    ProviderSignOut,
    ProviderUseAsPrimary,
    ProviderUseAsPrimaryConfirmTitle,
    ProviderUseAsPrimaryConfirmBody,
    ProviderTestConnection,
    ProviderSignInDialogTitle,
    ProviderSignInCopyUrl,
    ProviderSignInUrlLabel,
    ProviderImportClaudeCode,
    ProviderImportCodexCli,
    ProviderAddKey,
    ProviderAddSetupToken,
    ProviderStatusPrimary,
    ProviderStatusPrimaryNotConnected,
    ProviderStatusConnected,
    ProviderStatusKeyStored,
    ProviderStatusReconnect,
    ProviderStatusNotConnected,
    ProviderSignInBody,
    ProviderImportBody,
    ProviderModelsSearch,
    ProviderModelsMore,
    ProviderModelsLive,
    ProviderProbeResult,

    // ---- Channels -------------------------------------------------------
    ChannelsGroup,
    ChannelStatusWorking,
    ChannelStatusNotFinished,

    // ---- Integrations ---------------------------------------------------
    IntegrationsFilterInstalled,
    IntegrationsFilterAvailable,
    IntegrationsFilterMcps,
    IntegrationsFilterFeatures,
    IntegrationsSignInClients,
    IntegrationsConsentTitle,
    IntegrationsWhatLeaves,
    IntegrationsEmpty,
    IntegrationsSubtitle,
    IntegrationsSearchAccessible,
    IntegrationsSwitchAccessible,
    IntegrationsNextStep,
    IntegrationsClientRow,
    IntegrationsWorkspaceRow,
    IntegrationsOpen,
    CountedFilter,
    IntegrationsFeatureComputerUse,
    IntegrationsFeatureComputerUseBody,
    IntegrationsFeatureMeetings,
    IntegrationsFeatureMeetingsBody,
    OAuthClientDialogTitle,
    OAuthClientId,
    OAuthClientSecret,
    OAuthClientRedirectPort,
    OAuthClientRegion,
    WorkspaceDialogTitle,
    WorkspaceAccessProfile,

    // ---- Meetings -------------------------------------------------------
    MeetingsSharedSettings,
    MeetingsGoogleMeet,
    MeetingsZoom,
    MeetingsInstall,
    MeetingsSignIn,
    MeetingsSignInAgain,
    MeetingsSignInUnanswered,
    MeetingsSleepStatement,
    MeetingsEnableFooter,
    MeetingsGoogleAccount,

    // ---- Voice ----------------------------------------------------------
    VoiceCompanionStatement,
    VoiceMicrophoneStatement,

    // ---- Computer -------------------------------------------------------
    ComputerEnable,
    ComputerInstall,
    ComputerScreenCapture,
    ComputerInputControl,
    ComputerProbedAt,
    ComputerArm64,
    ComputerNoGraphicalSession,
    ComputerWaylandSession,
    ComputerSessionDetected,
    ComputerInstallStatement,
    ComputerVerdictAvailable,
    ComputerVerdictUnavailable,
    ComputerProbeUnread,

    // ---- Permissions ledger, section 7.4 --------------------------------
    PermissionsColumnRevoke,
    PermissionsColumnArtifact,
    PermissionsPlatformFact,
    PermissionMicrophoneTitle,
    PermissionMicrophonePrincipal,
    PermissionMicrophoneRevoke,
    PermissionMicrophoneArtifact,
    PermissionScreenCaptureTitle,
    PermissionScreenCapturePrincipal,
    PermissionScreenCaptureRevoke,
    PermissionScreenCaptureArtifact,
    PermissionInputSynthesisTitle,
    PermissionInputSynthesisPrincipal,
    PermissionInputSynthesisRevoke,
    PermissionInputSynthesisArtifact,
    PermissionBackgroundServiceTitle,
    PermissionBackgroundServicePrincipal,
    PermissionBackgroundServiceRevoke,
    PermissionBackgroundServiceArtifact,
    PermissionAutostartTitle,
    PermissionAutostartPrincipal,
    PermissionAutostartRevoke,
    PermissionAutostartArtifact,
    PermissionPrivilegedTitle,
    PermissionPrivilegedPrincipal,
    PermissionPrivilegedRevoke,
    PermissionPrivilegedArtifact,
    PermissionBrowserTitle,
    PermissionBrowserPrincipal,
    PermissionBrowserRevoke,
    PermissionBrowserArtifact,

    // ---- Doctor ---------------------------------------------------------
    DoctorSummaryHealthy,
    DoctorSummaryFailed,
    DoctorCheckedJustNow,
    DoctorRunNetworkChecks,
    DoctorNetworkChecksCost,
    DoctorExportSupportBundle,
    DoctorShowLogFolder,
    DoctorLogFolderUnavailable,
    DoctorEvidence,
    DoctorRunning,
    DoctorBusy,
    DoctorNeedsNewerEngine,
    DoctorDesktopSession,
    DoctorStatusPassed,
    DoctorStatusWarning,
    DoctorStatusFailed,
    DoctorStatusUnavailable,
    DoctorStatusSkipped,
    DoctorStatusCancelled,
    DoctorStatusTimedOut,
    DoctorStatusNotApplicable,
    DoctorPillPassed,
    DoctorPillWarning,
    DoctorPillFailed,
    DoctorPillUnavailable,
    DoctorPillSkipped,
    DoctorPillCancelled,
    DoctorPillTimedOut,
    DoctorPillNotApplicable,

    // ---- Logs -----------------------------------------------------------
    LogsLevel,
    LogsLevelAll,
    LogLevelEmergency,
    LogLevelAlert,
    LogLevelCritical,
    LogLevelError,
    LogLevelWarning,
    LogLevelNotice,
    LogLevelInfo,
    LogLevelDebug,
    LogsPause,
    LogsSearchAccessible,
    LogsCaptionFile,
    LogsCaptionJournal,
    LogsJournalCommand,
    LogsEmpty,
    LogsCursorExpired,
    LogsDaemonUnavailable,
    LogsExportDiagnostics,

    // ---- Jobs -----------------------------------------------------------
    JobPhaseCalling,
    JobPhaseBinding,
    JobPhaseAwaitingBrowser,
    JobPhaseVerifying,
    JobPhaseReadingKeychain,
    JobPhaseDownloading,
    JobPhaseProbing,
    JobPhaseListing,
    JobPhaseSidecarDownloading,
    JobPhaseAwaitingSignIn,
    JobCancelled,
    JobTimedOut,

    // ---- Recovery -------------------------------------------------------
    RecoveryConfigUnreadableTitle,
    RecoveryServiceRefusedTitle,
    RecoveryJournalDialogTitle,
    RecoveryEvidence,
    RecoveryRetry,
    RecoveryExportDiagnostics,
    RecoveryShowJournalCommand,
    RecoveryExportUnavailable,
    RecoveryPackageRepair,

    // ---- Setup assistant ------------------------------------------------
    SetupWelcomeTitle,
    SetupWelcomeBody,
    SetupWelcomeStart,
    SetupWelcomeChooseHome,
    SetupStartingTitle,
    SetupStepBindHome,
    SetupStepEnableLinger,
    SetupStepStartService,
    SetupStepVerifyDaemon,
    SetupStepReadSetupState,
    SetupConnectAiTitle,
    SetupConnectAiBody,
    SetupConnectAiSkip,
    SetupAboutYouTitle,
    SetupAboutYouBody,
    SetupFieldYourName,
    SetupFieldTimeZone,
    SetupFieldStyle,
    SetupFieldAssistantName,
    SetupApplyingTitle,
    SetupStepSaveSetup,
    SetupStepRestart,
    SetupReadyTitle,
    SetupReadyNext,
    SetupReadyChannels,
    SetupReadyAttention,
    SetupBootFailedTitle,
    SetupCancel,
    SetupChangeProvider,
    SetupBlockedProvider,
    SetupBlockedPersonalization,
    SetupBlockedElsewhere,
    SetupStepWaiting,
    SetupStepWorking,
    SetupStepDone,
    SetupStepFailed,
    SetupProgressAccessible,

    // ---- Setup, preflight refusals --------------------------------------
    SetupPreflightRefusedTitle,
    PreflightNoUserManagerTitle,
    PreflightNoUserManagerBody,
    PreflightForeignDaemonTitle,
    PreflightForeignDaemonBody,
    PreflightPreManagementDaemonTitle,
    PreflightPreManagementDaemonBody,
    PreflightEngineSkewTitle,
    PreflightEngineSkewBody,
    PreflightForeignDistributionTitle,
    PreflightForeignDistributionBody,

    // ---- Setup, activation failures -------------------------------------
    ActivationUnitInactiveTitle,
    ActivationUnitInactiveBody,
    ActivationCrashLoopTitle,
    ActivationCrashLoopBody,
    ActivationProtocolMismatchTitle,
    ActivationProtocolMismatchBody,
    ActivationBindFailureTitle,
    ActivationBindFailureBody,
    ActivationWebUnavailableTitle,
    ActivationWebUnavailableBody,
    ActivationLastLogLines,

    // ---- Version skew ---------------------------------------------------
    SkewNewerInstalledTitle,
    SkewNewerInstalledDetail,
    SkewRestartToFinish,
    SkewUnknownIdentityTitle,
    SkewUnknownIdentityBody,
    SkewStaleGuiTitle,
    SkewStaleGuiBody,
    SkewOwnershipConflictTitle,
    SkewOwnershipConflictBody,

    // ---- The Linux-only moments, M38 section 6.5 ------------------------
    LingerDeniedTitle,
    LingerDeniedBody,
    LingerDeniedCommandRow,
    LingerDeniedCommandValue,
    LoginManagerAbsentTitle,
    LoginManagerAbsentBody,
    LoginManagerAbsentNextAction,
    SecretStoreDesktopKeyring,
    SecretStorePasswordStore,
    SecretStoreNone,
    SecretStoreUnavailableTitle,
    SecretStoreUnavailableBody,
    SecretStoreUnavailableNextAction,
    SystemScopeRefusalTitle,
    SystemScopeRefusalBody,
    SystemScopeRefusalNextAction,
    SystemScopeRefusalCommandValue,

    // ---- The one notification this application raises --------------------
    NoticeAttentionTitle,
    NoticeAttentionBody,

    /// Not a key. The number of keys, which is what makes the catalogue gate
    /// total rather than nearly total.
    Count,
}

/// The catalogue, in [`Key`] order.
pub const CATALOGUE: &[Entry] = &[
    row(Key::ProductName, "Fermix", Casing::Header),
    // Page titles.
    row(Key::PageHome, "Home", Casing::Header),
    row(Key::PageDoctor, "Doctor", Casing::Header),
    row(Key::PageLogs, "Logs", Casing::Header),
    row(Key::PageSettings, "Settings", Casing::Header),
    row(Key::PageRecovery, "Recovery", Casing::Header),
    row(Key::PageSetup, "Set up Fermix", Casing::Header),
    // Primary menu.
    row(Key::MenuSettings, "Settings\u{2026}", Casing::Header),
    row(Key::MenuRunDoctor, "Run doctor", Casing::Header),
    row(Key::MenuRestart, "Restart Fermix\u{2026}", Casing::Header),
    row(Key::MenuKeyboardShortcuts, "Keyboard shortcuts", Casing::Header),
    row(Key::MenuAbout, "About Fermix", Casing::Header),
    row(Key::MenuQuit, "Quit", Casing::Header),
    row(Key::MenuPrimaryAccessible, "Main menu", Casing::Header),
    // Navigation.
    row(Key::BackToFermix, "Back to Fermix", Casing::Header),
    row(Key::ToggleSidebarAccessible, "Show or hide the sidebar", Casing::Header),
    row(Key::SidebarAccessible, "Sections", Casing::Header),
    // Shortcuts dialog.
    row(Key::ShortcutsTitle, "Keyboard shortcuts", Casing::Header),
    row(Key::ShortcutsGroupGeneral, "General", Casing::Header),
    row(Key::ShortcutsGroupNavigation, "Navigation", Casing::Header),
    row(Key::ShortcutsGroupActions, "Actions", Casing::Header),
    row(Key::ShortcutOpenSettings, "Open settings", Casing::Header),
    row(Key::ShortcutShowHome, "Show home", Casing::Header),
    row(Key::ShortcutShowDoctor, "Show doctor", Casing::Header),
    row(Key::ShortcutShowLogs, "Show logs", Casing::Header),
    row(Key::ShortcutGoBack, "Go back", Casing::Header),
    row(Key::ShortcutSearchSurface, "Search this surface", Casing::Header),
    row(Key::ShortcutRunDoctor, "Run doctor", Casing::Header),
    row(Key::ShortcutRestart, "Restart Fermix", Casing::Header),
    row(Key::ShortcutToggleSidebar, "Show or hide the sidebar", Casing::Header),
    row(Key::ShortcutCloseWindow, "Close the window", Casing::Header),
    row(Key::ShortcutQuit, "Quit Fermix", Casing::Header),
    row(Key::ShortcutAbout, "About Fermix", Casing::Header),
    row(Key::ShortcutKeyboardShortcuts, "Keyboard shortcuts", Casing::Header),
    // About.
    row(Key::AboutDeveloper, "Tezra", Casing::Sentence),
    row(
        Key::AboutComments,
        "Fermix runs as a background service on this computer. This window sets it up and keeps an eye on it.",
        Casing::Sentence,
    ),
    row(Key::AboutWebsite, "Website", Casing::Header),
    // Shared controls.
    row(Key::ActionCancel, "Cancel", Casing::Header),
    row(Key::ActionClose, "Close", Casing::Header),
    row(Key::ActionContinue, "Continue", Casing::Header),
    row(Key::ActionBack, "Back", Casing::Header),
    row(Key::ActionTryAgain, "Try again", Casing::Header),
    row(Key::ActionRefresh, "Refresh", Casing::Header),
    row(Key::ActionExport, "Export", Casing::Header),
    row(Key::ActionCopyCommand, "Copy command", Casing::Header),
    row(Key::ActionCopyCommands, "Copy commands", Casing::Header),
    row(Key::ActionShowMeHow, "Show me how", Casing::Header),
    row(Key::ActionOpenSettingsPane, "Open settings", Casing::Header),
    row(Key::ActionSearchAccessible, "Search", Casing::Header),
    row(Key::ActionMoreAccessible, "More actions", Casing::Header),
    row(
        Key::BusyQueueFull,
        "Fermix is doing as much at once as it can. Try that again in a moment.",
        Casing::Sentence,
    ),
    row(Key::StateOn, "On", Casing::Sentence),
    row(Key::StateOff, "Off", Casing::Sentence),
    row(Key::StateNotReported, "Not reported", Casing::Sentence),
    // Home.
    row(Key::HomeGroupBackground, "Background", Casing::Header),
    row(Key::HomeGroupAttention, "Attention", Casing::Header),
    row(Key::HomeGroupRuntimeDetails, "Runtime details", Casing::Header),
    row(Key::HomeRowStatus, "Status", Casing::Sentence),
    row(Key::HomeStatusRunning, "Running", Casing::Sentence),
    row(Key::HomeStatusSetupRequired, "Setup required", Casing::Sentence),
    row(
        Key::HomeStatusRestartToFinishUpdating,
        "Restart to finish updating",
        Casing::Sentence,
    ),
    row(Key::HomeStatusNotRunning, "Fermix isn\u{2019}t running", Casing::Sentence),
    row(Key::HomeSwitchRunInBackground, "Run in the background", Casing::Sentence),
    row(Key::HomeSwitchOpenAtLogin, "Open at login", Casing::Sentence),
    row(Key::HomeAttentionEmpty, "Nothing needs your attention", Casing::Sentence),
    row(Key::HomeRuntimeEngine, "Engine", Casing::Sentence),
    row(Key::HomeRuntimeManagementProtocol, "Management protocol", Casing::Sentence),
    row(Key::HomeRuntimeUptime, "Uptime", Casing::Sentence),
    row(Key::HomeRuntimeProvider, "Provider", Casing::Sentence),
    row(Key::HomeRuntimeChannels, "Channels", Casing::Sentence),
    row(Key::HomeRuntimeSkills, "Skills", Casing::Sentence),
    row(Key::HomeRuntimeTools, "Tools", Casing::Sentence),
    row(Key::HomeRuntimeService, "Service", Casing::Sentence),
    row(Key::HomeRuntimeSession, "Session", Casing::Sentence),
    row(Key::HomeRuntimeUnavailable, "Unavailable", Casing::Sentence),
    row(Key::HomeRuntimeNone, "None", Casing::Sentence),
    row(Key::HomeUptimeDaysHours, "{days} d {hours} h", Casing::Sentence),
    row(Key::HomeUptimeHoursMinutes, "{hours} h {minutes} min", Casing::Sentence),
    row(Key::HomeUptimeMinutes, "{minutes} min", Casing::Sentence),
    row(Key::HomeUptimeJustStarted, "Just started", Casing::Sentence),
    row(Key::HomeToolbarContinueSetup, "Continue setup", Casing::Header),
    row(Key::HomeToolbarFinishUpdating, "Finish updating", Casing::Header),
    // Attention rows, one template per detail_key family.
    row(
        Key::AttentionProviderTitle,
        "Fermix has no AI provider it can use",
        Casing::Sentence,
    ),
    row(Key::AttentionProviderAction, "Open providers", Casing::Header),
    row(
        Key::AttentionPersonalizationTitle,
        "Fermix does not know who it is working for yet",
        Casing::Sentence,
    ),
    row(Key::AttentionPersonalizationAction, "Open personality", Casing::Header),
    row(
        Key::AttentionChannelTitle,
        "A channel is switched on but not finished",
        Casing::Sentence,
    ),
    row(Key::AttentionChannelAction, "Open channels", Casing::Header),
    row(
        Key::AttentionRealtimeTitle,
        "Voice is switched on but not finished",
        Casing::Sentence,
    ),
    row(Key::AttentionRealtimeAction, "Open voice", Casing::Header),
    row(
        Key::AttentionRestartPendingTitle,
        "Some settings take effect the next time Fermix starts",
        Casing::Sentence,
    ),
    row(
        Key::AttentionLegacyServiceUnitTitle,
        "An older service file is starting Fermix",
        Casing::Sentence,
    ),
    row(
        Key::AttentionEnginePathBaselineTitle,
        "Fermix may not find the tools you installed",
        Casing::Sentence,
    ),
    row(Key::AttentionOpenDoctorAction, "Open doctor", Casing::Header),
    // Restart.
    row(Key::RestartDialogTitle, "Restart Fermix", Casing::Header),
    row(
        Key::RestartDialogBody,
        "Fermix stops and starts again. Your settings and conversations are kept.",
        Casing::Sentence,
    ),
    row(
        Key::RestartConversationsKnown,
        "Conversations in progress: {count}",
        Casing::Sentence,
    ),
    row(
        Key::RestartConversationsUnknown,
        "Fermix cannot tell how many conversations are in progress.",
        Casing::Sentence,
    ),
    row(Key::RestartNow, "Restart now", Casing::Header),
    row(Key::RestartWhenIdle, "Restart when idle", Casing::Header),
    row(Key::FinishUpdatingDialogTitle, "Finish updating", Casing::Header),
    row(
        Key::FinishUpdatingDialogBody,
        "Restarting moves the background service onto the version already installed on this computer.",
        Casing::Sentence,
    ),
    // Banners.
    row(
        Key::BannerExternalChangeTitle,
        "Settings changed outside Fermix",
        Casing::Sentence,
    ),
    row(Key::BannerReloadFromDisk, "Reload settings from disk", Casing::Header),
    // Settings groups and panes.
    row(Key::SettingsGroupAssistant, "Assistant", Casing::Header),
    row(Key::SettingsGroupConnections, "Connections", Casing::Header),
    row(Key::SettingsGroupCapabilities, "Capabilities", Casing::Header),
    row(Key::SettingsGroupSystem, "System", Casing::Header),
    row(Key::PaneProviders, "Providers", Casing::Header),
    row(Key::PanePersonality, "Personality", Casing::Header),
    row(Key::PaneMemory, "Memory", Casing::Header),
    row(Key::PaneChannels, "Channels", Casing::Header),
    row(Key::PaneIntegrations, "Integrations", Casing::Header),
    row(Key::PaneVoice, "Voice", Casing::Header),
    row(Key::PaneMeetings, "Meetings", Casing::Header),
    row(Key::PaneComputer, "Computer", Casing::Header),
    row(Key::PaneCodingAgents, "Coding agents", Casing::Header),
    row(Key::PaneSearch, "Search", Casing::Header),
    row(Key::PaneImages, "Images", Casing::Header),
    row(Key::PaneSandbox, "Sandbox", Casing::Header),
    row(Key::PanePermissions, "Permissions", Casing::Header),
    row(Key::SettingsSearchAccessible, "Search the settings panes", Casing::Header),
    row(Key::SettingsSearchEmpty, "No pane matches that search", Casing::Sentence),
    row(Key::ListAdd, "Add an item", Casing::Header),
    row(Key::ListRemoveAccessible, "Remove this item", Casing::Header),
    // Secrets.
    row(Key::SecretStored, "Stored", Casing::Sentence),
    row(Key::SecretReplace, "Replace\u{2026}", Casing::Header),
    row(Key::SecretRemove, "Remove", Casing::Header),
    row(Key::SecretAdd, "Add\u{2026}", Casing::Header),
    row(Key::SecretDialogTitleAdd, "Add a key", Casing::Header),
    row(Key::SecretDialogTitleReplace, "Replace the key", Casing::Header),
    row(Key::SecretFieldLabel, "Key", Casing::Sentence),
    row(Key::SecretStoreRowLabel, "Secret store", Casing::Sentence),
    row(Key::SecretDialogConfirm, "Store the key", Casing::Header),
    // Providers.
    row(Key::ProvidersGroupPrimary, "Primary", Casing::Header),
    row(Key::ProvidersGroupProviders, "Providers", Casing::Header),
    row(Key::ProviderChooseModel, "Choose a model\u{2026}", Casing::Header),
    row(Key::ProviderModelDialogTitle, "Choose a model", Casing::Header),
    row(Key::ProviderSignIn, "Sign in", Casing::Header),
    row(Key::ProviderSignOut, "Sign out", Casing::Header),
    row(Key::ProviderUseAsPrimary, "Use as primary", Casing::Header),
    row(
        Key::ProviderUseAsPrimaryConfirmTitle,
        "Use this provider for everything",
        Casing::Header,
    ),
    row(
        Key::ProviderUseAsPrimaryConfirmBody,
        "Fermix answers with this provider from now on. The model and the reasoning effort move with it.",
        Casing::Sentence,
    ),
    row(Key::ProviderTestConnection, "Test the connection", Casing::Header),
    row(Key::ProviderSignInDialogTitle, "Sign in", Casing::Header),
    row(Key::ProviderSignInCopyUrl, "Copy the address", Casing::Header),
    row(
        Key::ProviderSignInUrlLabel,
        "If your browser did not open, use this address:",
        Casing::Sentence,
    ),
    row(Key::ProviderImportClaudeCode, "Use the Claude Code sign-in", Casing::Header),
    row(Key::ProviderImportCodexCli, "Use the Codex sign-in", Casing::Header),
    row(Key::ProviderAddKey, "Add key\u{2026}", Casing::Header),
    row(Key::ProviderAddSetupToken, "Add setup token\u{2026}", Casing::Header),
    row(Key::ProviderStatusPrimary, "Primary", Casing::Sentence),
    row(
        Key::ProviderStatusPrimaryNotConnected,
        "Primary, not connected",
        Casing::Sentence,
    ),
    row(Key::ProviderStatusConnected, "Connected", Casing::Sentence),
    row(Key::ProviderStatusKeyStored, "Key stored, not checked", Casing::Sentence),
    row(Key::ProviderStatusReconnect, "Reconnect needed", Casing::Sentence),
    row(Key::ProviderStatusNotConnected, "Not connected", Casing::Sentence),
    row(
        Key::ProviderSignInBody,
        "Finish signing in in your browser. Fermix never sees your password.",
        Casing::Sentence,
    ),
    row(
        Key::ProviderImportBody,
        "Fermix is reading the sign-in this computer already holds. Nothing is sent anywhere.",
        Casing::Sentence,
    ),
    row(Key::ProviderModelsSearch, "Search models", Casing::Sentence),
    row(Key::ProviderModelsMore, "Show more", Casing::Header),
    row(Key::ProviderModelsLive, "Ask the provider", Casing::Header),
    row(
        Key::ProviderProbeResult,
        "Answered in {count} ms with {model}",
        Casing::Sentence,
    ),
    // Channels.
    row(Key::ChannelsGroup, "Channels", Casing::Header),
    row(Key::ChannelStatusWorking, "Connected", Casing::Sentence),
    row(Key::ChannelStatusNotFinished, "Not finished", Casing::Sentence),
    // Integrations.
    row(Key::IntegrationsFilterInstalled, "Installed", Casing::Header),
    row(Key::IntegrationsFilterAvailable, "Available", Casing::Header),
    row(Key::IntegrationsFilterMcps, "MCPs", Casing::Header),
    row(Key::IntegrationsFilterFeatures, "Features", Casing::Header),
    row(Key::IntegrationsSignInClients, "Sign-in clients", Casing::Header),
    row(Key::IntegrationsConsentTitle, "Add this integration", Casing::Header),
    row(Key::IntegrationsWhatLeaves, "What leaves this computer", Casing::Header),
    row(
        Key::IntegrationsEmpty,
        "No integration matches that search",
        Casing::Sentence,
    ),
    row(
        Key::IntegrationsSubtitle,
        "Plugins, servers and the built-in drivers Fermix can use.",
        Casing::Sentence,
    ),
    row(Key::IntegrationsSearchAccessible, "Search integrations", Casing::Header),
    row(
        Key::IntegrationsSwitchAccessible,
        "Turn {name} on or off",
        Casing::Sentence,
    ),
    row(Key::IntegrationsNextStep, "Next step", Casing::Sentence),
    row(Key::IntegrationsClientRow, "Sign-in client", Casing::Sentence),
    row(Key::IntegrationsWorkspaceRow, "Workspace", Casing::Sentence),
    row(Key::IntegrationsOpen, "Open", Casing::Header),
    row(Key::CountedFilter, "{name} ({count})", Casing::Sentence),
    row(Key::IntegrationsFeatureComputerUse, "Computer use", Casing::Header),
    row(
        Key::IntegrationsFeatureComputerUseBody,
        "Sees this screen and drives the keyboard and pointer.",
        Casing::Sentence,
    ),
    row(Key::IntegrationsFeatureMeetings, "Meeting notetaker", Casing::Header),
    row(
        Key::IntegrationsFeatureMeetingsBody,
        "Joins a meeting as a notetaker and writes the notes up afterwards.",
        Casing::Sentence,
    ),
    row(Key::OAuthClientDialogTitle, "Sign-in client", Casing::Header),
    row(Key::OAuthClientId, "Client identifier", Casing::Sentence),
    row(Key::OAuthClientSecret, "Client secret", Casing::Sentence),
    row(Key::OAuthClientRedirectPort, "Redirect port", Casing::Sentence),
    row(Key::OAuthClientRegion, "Region", Casing::Sentence),
    row(Key::WorkspaceDialogTitle, "Choose a workspace", Casing::Header),
    row(Key::WorkspaceAccessProfile, "Access", Casing::Sentence),
    // Meetings.
    row(Key::MeetingsSharedSettings, "Shared settings", Casing::Header),
    row(Key::MeetingsGoogleMeet, "Google Meet", Casing::Header),
    row(Key::MeetingsZoom, "Zoom", Casing::Header),
    row(Key::MeetingsInstall, "Install the notetaker", Casing::Header),
    row(Key::MeetingsSignIn, "Sign in", Casing::Header),
    row(Key::MeetingsSignInAgain, "Sign in again", Casing::Header),
    row(
        Key::MeetingsSignInUnanswered,
        "Fermix could not read the sign-in state on this computer.",
        Casing::Sentence,
    ),
    row(
        Key::MeetingsSleepStatement,
        "Fermix cannot keep this computer awake during a meeting. If the machine suspends, the recording stops. Adjust your power settings before a long meeting.",
        Casing::Sentence,
    ),
    row(
        Key::MeetingsEnableFooter,
        "Installs the notetaker and its browser on first enable, about 150 MB.",
        Casing::Sentence,
    ),
    row(
        Key::MeetingsGoogleAccount,
        "Google account for the notetaker",
        Casing::Sentence,
    ),
    // Voice.
    row(
        Key::VoiceCompanionStatement,
        "Voice is configured here and used from a companion application. The companion exists for macOS today, and there is no Linux companion yet.",
        Casing::Sentence,
    ),
    row(
        Key::VoiceMicrophoneStatement,
        "Linux has no microphone permission. Nothing asked you, nothing appears in your system settings, and there is nothing to revoke. While Fermix is running it can open the microphone at any time, and so can any other program you run. Your real controls are to not run it, to mute the microphone in your sound settings or in PipeWire, or to run it in a sandbox that withholds audio, which also stops it playing sound. On macOS the operating system asks first. On Linux it does not.",
        Casing::Sentence,
    ),
    // Computer.
    row(Key::ComputerEnable, "Let Fermix use this computer", Casing::Sentence),
    row(Key::ComputerInstall, "Install the helper", Casing::Header),
    row(Key::ComputerScreenCapture, "Screen capture", Casing::Sentence),
    row(Key::ComputerInputControl, "Input control", Casing::Sentence),
    row(Key::ComputerProbedAt, "Checked {time}", Casing::Sentence),
    row(
        Key::ComputerArm64,
        "Computer use is not available on this architecture. Fermix runs fully on arm64 Linux, but the computer-use sidecar has no arm64 build, so there is nothing to install.",
        Casing::Sentence,
    ),
    row(
        Key::ComputerNoGraphicalSession,
        "Computer use needs a graphical session. This host has no display server running, so there is nothing to see and nothing to drive.",
        Casing::Sentence,
    ),
    row(
        Key::ComputerWaylandSession,
        "Computer use is not available on this Wayland session. The sidecar drives the screen and keyboard through X11, and this session does not offer one.",
        Casing::Sentence,
    ),
    row(Key::ComputerSessionDetected, "Session detected: {session}", Casing::Sentence),
    row(
        Key::ComputerInstallStatement,
        "Fermix downloads the helper from its own release and installs it for this account. Its size is shown while it downloads.",
        Casing::Sentence,
    ),
    row(Key::ComputerVerdictAvailable, "Available", Casing::Sentence),
    row(Key::ComputerVerdictUnavailable, "Not available", Casing::Sentence),
    row(Key::ComputerProbeUnread, "Not checked yet", Casing::Sentence),
    // Permissions ledger.
    // Two column labels, not three: the principal is the row's own subtitle,
    // and the other two sit inside the expander where they need naming.
    row(Key::PermissionsColumnRevoke, "How you take it back", Casing::Sentence),
    row(Key::PermissionsColumnArtifact, "Where it is kept", Casing::Sentence),
    row(
        Key::PermissionsPlatformFact,
        "On this platform a permission is something Fermix asserts about itself and keeps for itself, not something the operating system verifies and stores. Lose what Fermix keeps and the consent is gone; copy it and the consent moves with it.",
        Casing::Sentence,
    ),
    row(Key::PermissionMicrophoneTitle, "Microphone and voice", Casing::Sentence),
    row(
        Key::PermissionMicrophonePrincipal,
        "Nobody. PipeWire serves whoever asks.",
        Casing::Sentence,
    ),
    row(
        Key::PermissionMicrophoneRevoke,
        "There is nothing to take back. Quit the program, or mute the microphone in PipeWire or in your sound settings.",
        Casing::Sentence,
    ),
    row(
        Key::PermissionMicrophoneArtifact,
        "Nowhere. Nothing records that access happened.",
        Casing::Sentence,
    ),
    row(Key::PermissionScreenCaptureTitle, "Screen capture", Casing::Sentence),
    row(
        Key::PermissionScreenCapturePrincipal,
        "A screen cast session tied to the desktop portal identity this program presents.",
        Casing::Sentence,
    ),
    row(
        Key::PermissionScreenCaptureRevoke,
        "Your desktop\u{2019}s sharing settings.",
        Casing::Sentence,
    ),
    row(
        Key::PermissionScreenCaptureArtifact,
        "A restore token the helper writes and replaces on every session.",
        Casing::Sentence,
    ),
    row(Key::PermissionInputSynthesisTitle, "Keyboard and pointer control", Casing::Sentence),
    row(
        Key::PermissionInputSynthesisPrincipal,
        "A remote desktop session with input devices, taken together with any capture sources it carries.",
        Casing::Sentence,
    ),
    row(
        Key::PermissionInputSynthesisRevoke,
        "Your desktop\u{2019}s sharing settings.",
        Casing::Sentence,
    ),
    row(
        Key::PermissionInputSynthesisArtifact,
        "One restore token for the whole session, held by the helper.",
        Casing::Sentence,
    ),
    row(Key::PermissionBackgroundServiceTitle, "Background service", Casing::Sentence),
    row(
        Key::PermissionBackgroundServicePrincipal,
        "Your own user service manager, plus permission to keep it running after you log out.",
        Casing::Sentence,
    ),
    row(
        Key::PermissionBackgroundServiceRevoke,
        "Turn the background service off on the Home page.",
        Casing::Sentence,
    ),
    row(
        Key::PermissionBackgroundServiceArtifact,
        "The packaged service file, your own enablement of it, and the home it is bound to.",
        Casing::Sentence,
    ),
    row(Key::PermissionAutostartTitle, "Open at login", Casing::Sentence),
    row(
        Key::PermissionAutostartPrincipal,
        "A desktop entry in your own home directory.",
        Casing::Sentence,
    ),
    row(
        Key::PermissionAutostartRevoke,
        "Turn off opening at login on the Home page, or remove the entry yourself.",
        Casing::Sentence,
    ),
    row(
        Key::PermissionAutostartArtifact,
        "The entry itself, in your own home directory.",
        Casing::Sentence,
    ),
    row(Key::PermissionPrivilegedTitle, "Administrator actions", Casing::Sentence),
    row(
        Key::PermissionPrivilegedPrincipal,
        "Your system\u{2019}s authorization service, asked only when you run one of the commands Fermix prints.",
        Casing::Sentence,
    ),
    row(
        Key::PermissionPrivilegedRevoke,
        "Nothing is kept, so there is nothing to take back. A granted answer is commonly remembered for a few minutes.",
        Casing::Sentence,
    ),
    row(
        Key::PermissionPrivilegedArtifact,
        "Nowhere. Fermix installs no authorization rule of its own.",
        Casing::Sentence,
    ),
    row(Key::PermissionBrowserTitle, "Browser automation", Casing::Sentence),
    row(
        Key::PermissionBrowserPrincipal,
        "On Ubuntu, the confinement profile that ships for the browser at its packaged path.",
        Casing::Sentence,
    ),
    row(
        Key::PermissionBrowserRevoke,
        "Replace or remove that profile, which is a decision for the whole machine.",
        Casing::Sentence,
    ),
    row(
        Key::PermissionBrowserArtifact,
        "A profile on the host, keyed to the browser\u{2019}s path. A browser installed elsewhere is not covered by it.",
        Casing::Sentence,
    ),
    // Doctor.
    row(Key::DoctorSummaryHealthy, "Healthy", Casing::Sentence),
    row(Key::DoctorSummaryFailed, "{count} failed", Casing::Sentence),
    row(Key::DoctorCheckedJustNow, "Checked just now", Casing::Sentence),
    row(Key::DoctorRunNetworkChecks, "Run network checks", Casing::Header),
    row(
        Key::DoctorNetworkChecksCost,
        "Network checks call the services you have already configured.",
        Casing::Sentence,
    ),
    row(Key::DoctorExportSupportBundle, "Export support bundle", Casing::Header),
    row(Key::DoctorShowLogFolder, "Show log folder", Casing::Header),
    row(
        Key::DoctorLogFolderUnavailable,
        "Fermix has not said where its files are, so there is no folder to open.",
        Casing::Sentence,
    ),
    row(Key::DoctorEvidence, "Evidence", Casing::Header),
    row(Key::DoctorRunning, "Checks are running", Casing::Sentence),
    row(
        Key::DoctorBusy,
        "Another run is already in progress, so this one is waiting.",
        Casing::Sentence,
    ),
    row(
        Key::DoctorNeedsNewerEngine,
        "This check needs a newer background service than the one running.",
        Casing::Sentence,
    ),
    row(Key::DoctorDesktopSession, "This desktop session", Casing::Header),
    row(Key::DoctorStatusPassed, "Passed", Casing::Sentence),
    row(Key::DoctorStatusWarning, "Warning", Casing::Sentence),
    row(Key::DoctorStatusFailed, "Failed", Casing::Sentence),
    row(Key::DoctorStatusUnavailable, "Unavailable", Casing::Sentence),
    row(Key::DoctorStatusSkipped, "Skipped", Casing::Sentence),
    row(Key::DoctorStatusCancelled, "Cancelled", Casing::Sentence),
    row(Key::DoctorStatusTimedOut, "Timed out", Casing::Sentence),
    row(Key::DoctorStatusNotApplicable, "Not applicable", Casing::Sentence),
    row(Key::DoctorPillPassed, "P", Casing::Sentence),
    row(Key::DoctorPillWarning, "W", Casing::Sentence),
    row(Key::DoctorPillFailed, "F", Casing::Sentence),
    row(Key::DoctorPillUnavailable, "U", Casing::Sentence),
    row(Key::DoctorPillSkipped, "S", Casing::Sentence),
    row(Key::DoctorPillCancelled, "C", Casing::Sentence),
    row(Key::DoctorPillTimedOut, "T", Casing::Sentence),
    row(Key::DoctorPillNotApplicable, "N", Casing::Sentence),
    // Logs.
    row(Key::LogsLevel, "Level", Casing::Sentence),
    row(Key::LogsLevelAll, "All levels", Casing::Sentence),
    row(Key::LogLevelEmergency, "Emergency", Casing::Sentence),
    row(Key::LogLevelAlert, "Alert", Casing::Sentence),
    row(Key::LogLevelCritical, "Critical", Casing::Sentence),
    row(Key::LogLevelError, "Error", Casing::Sentence),
    row(Key::LogLevelWarning, "Warning", Casing::Sentence),
    row(Key::LogLevelNotice, "Notice", Casing::Sentence),
    row(Key::LogLevelInfo, "Info", Casing::Sentence),
    row(Key::LogLevelDebug, "Debug", Casing::Sentence),
    row(Key::LogsPause, "Pause", Casing::Header),
    row(Key::LogsSearchAccessible, "Search the log", Casing::Header),
    row(Key::LogsCaptionFile, "Fermix writes this log to {path}", Casing::Sentence),
    row(
        Key::LogsCaptionJournal,
        "Everything the service writes before this log opens goes to the journal instead.",
        Casing::Sentence,
    ),
    row(Key::LogsJournalCommand, "journalctl --user -u fermix", Casing::Sentence),
    row(Key::LogsEmpty, "No entry matches", Casing::Sentence),
    row(
        Key::LogsCursorExpired,
        "The log rotated while you were reading it, so this view starts again at the newest entries.",
        Casing::Sentence,
    ),
    row(
        Key::LogsDaemonUnavailable,
        "Fermix is not running, so there is nothing to read here.",
        Casing::Sentence,
    ),
    row(Key::LogsExportDiagnostics, "Export diagnostics", Casing::Header),
    // Jobs.
    row(Key::JobPhaseCalling, "Calling the provider", Casing::Sentence),
    row(Key::JobPhaseBinding, "Getting ready", Casing::Sentence),
    row(Key::JobPhaseAwaitingBrowser, "Waiting for your browser", Casing::Sentence),
    row(Key::JobPhaseVerifying, "Checking what came back", Casing::Sentence),
    row(Key::JobPhaseReadingKeychain, "Reading the stored sign-in", Casing::Sentence),
    row(Key::JobPhaseDownloading, "Downloading", Casing::Sentence),
    row(Key::JobPhaseProbing, "Checking the connection", Casing::Sentence),
    row(Key::JobPhaseListing, "Listing what it can reach", Casing::Sentence),
    row(Key::JobPhaseSidecarDownloading, "Downloading the helper", Casing::Sentence),
    row(Key::JobPhaseAwaitingSignIn, "Waiting for you to sign in", Casing::Sentence),
    row(Key::JobCancelled, "Cancelled", Casing::Sentence),
    row(Key::JobTimedOut, "Timed out", Casing::Sentence),
    // Recovery.
    row(
        Key::RecoveryConfigUnreadableTitle,
        "Fermix cannot read its settings",
        Casing::Header,
    ),
    row(
        Key::RecoveryServiceRefusedTitle,
        "Fermix could not be started",
        Casing::Header,
    ),
    row(Key::RecoveryJournalDialogTitle, "Read the journal", Casing::Header),
    row(Key::RecoveryEvidence, "What Fermix found", Casing::Header),
    row(Key::RecoveryRetry, "Try again", Casing::Header),
    row(Key::RecoveryExportDiagnostics, "Export diagnostics", Casing::Header),
    row(Key::RecoveryShowJournalCommand, "Show the journal command", Casing::Header),
    row(
        Key::RecoveryExportUnavailable,
        "The Fermix command line could not run, so there is nothing to export from here.",
        Casing::Sentence,
    ),
    row(
        Key::RecoveryPackageRepair,
        "Reinstall the packages with your package manager, then try again.",
        Casing::Sentence,
    ),
    // Setup assistant.
    row(Key::SetupWelcomeTitle, "Welcome to Fermix", Casing::Header),
    row(
        Key::SetupWelcomeBody,
        "Fermix runs on this computer, answers on the apps you already use, and keeps what it learns here.",
        Casing::Sentence,
    ),
    row(Key::SetupWelcomeStart, "Get started", Casing::Header),
    row(Key::SetupWelcomeChooseHome, "Use an existing Fermix folder", Casing::Header),
    row(Key::SetupStartingTitle, "Starting Fermix", Casing::Header),
    row(Key::SetupStepBindHome, "Choosing where Fermix keeps its files", Casing::Sentence),
    row(Key::SetupStepEnableLinger, "Getting permission to stay running", Casing::Sentence),
    row(Key::SetupStepStartService, "Starting the background service", Casing::Sentence),
    row(Key::SetupStepVerifyDaemon, "Checking that it answers", Casing::Sentence),
    row(Key::SetupStepReadSetupState, "Reading what is left to do", Casing::Sentence),
    row(Key::SetupConnectAiTitle, "Connect your AI", Casing::Header),
    row(
        Key::SetupConnectAiBody,
        "Fermix needs one model provider to answer with. You can add more later.",
        Casing::Sentence,
    ),
    row(Key::SetupConnectAiSkip, "Do this later", Casing::Header),
    row(Key::SetupAboutYouTitle, "About you", Casing::Header),
    row(
        Key::SetupAboutYouBody,
        "None of this is required. Fermix works without it and you can change it whenever you like.",
        Casing::Sentence,
    ),
    row(Key::SetupFieldYourName, "Your name", Casing::Sentence),
    row(Key::SetupFieldTimeZone, "Time zone", Casing::Sentence),
    row(Key::SetupFieldStyle, "Style", Casing::Sentence),
    row(Key::SetupFieldAssistantName, "What to call Fermix", Casing::Sentence),
    row(Key::SetupApplyingTitle, "Applying", Casing::Header),
    row(Key::SetupStepSaveSetup, "Saving the setup", Casing::Sentence),
    row(Key::SetupStepRestart, "Restarting Fermix", Casing::Sentence),
    row(Key::SetupReadyTitle, "Fermix is live", Casing::Header),
    row(Key::SetupReadyNext, "Next, if you like", Casing::Header),
    row(
        Key::SetupReadyChannels,
        "Connect Telegram, Slack or Discord",
        Casing::Sentence,
    ),
    row(
        Key::SetupReadyAttention,
        "Some things still need attention",
        Casing::Sentence,
    ),
    row(Key::SetupBootFailedTitle, "Fermix did not start", Casing::Header),
    row(Key::SetupCancel, "Stop setting up", Casing::Header),
    row(Key::SetupChangeProvider, "Change provider", Casing::Header),
    row(
        Key::SetupBlockedProvider,
        "Connect one provider before going on.",
        Casing::Sentence,
    ),
    row(
        Key::SetupBlockedPersonalization,
        "Fermix still needs something about you before it can finish.",
        Casing::Sentence,
    ),
    row(
        Key::SetupBlockedElsewhere,
        "Something in settings still needs an answer before Fermix is ready.",
        Casing::Sentence,
    ),
    row(Key::SetupStepWaiting, "Waiting", Casing::Sentence),
    row(Key::SetupStepWorking, "Working", Casing::Sentence),
    row(Key::SetupStepDone, "Done", Casing::Sentence),
    row(Key::SetupStepFailed, "Stopped here", Casing::Sentence),
    row(
        Key::SetupProgressAccessible,
        "Step {step} of {total}",
        Casing::Sentence,
    ),
    // Setup, preflight refusals.
    row(
        Key::SetupPreflightRefusedTitle,
        "Fermix cannot start setting up",
        Casing::Header,
    ),
    row(
        Key::PreflightNoUserManagerTitle,
        "This host has no user service manager",
        Casing::Header,
    ),
    row(
        Key::PreflightNoUserManagerBody,
        "Fermix keeps running in the background through your own service manager, and this host does not have one. Nothing was installed and your settings are unchanged.",
        Casing::Sentence,
    ),
    row(
        Key::PreflightForeignDaemonTitle,
        "Another Fermix is already running here",
        Casing::Header,
    ),
    row(
        Key::PreflightForeignDaemonBody,
        "A Fermix this app does not manage is using this account. Nothing was changed. Stop it first, then start again.",
        Casing::Sentence,
    ),
    row(
        Key::PreflightPreManagementDaemonTitle,
        "The running Fermix is too old to manage from here",
        Casing::Header,
    ),
    row(
        Key::PreflightPreManagementDaemonBody,
        "The Fermix running on this account was built before this window could talk to it. Nothing was changed. Update the packages, then start again.",
        Casing::Sentence,
    ),
    row(Key::PreflightEngineSkewTitle, "A newer Fermix is installed", Casing::Header),
    row(
        Key::PreflightEngineSkewBody,
        "The background service is still running the previous version. Restart it before setting up, so the setup lands on the version you installed.",
        Casing::Sentence,
    ),
    row(
        Key::PreflightForeignDistributionTitle,
        "This Fermix was not installed from a package",
        Casing::Header,
    ),
    row(
        Key::PreflightForeignDistributionBody,
        "The Fermix running here came from somewhere this app does not manage, so it can be read but not changed. Nothing was changed.",
        Casing::Sentence,
    ),
    // Setup, activation failures.
    row(Key::ActivationUnitInactiveTitle, "Fermix started and stopped", Casing::Header),
    row(
        Key::ActivationUnitInactiveBody,
        "The background service is switched on but is not running. Nothing else was changed.",
        Casing::Sentence,
    ),
    row(Key::ActivationCrashLoopTitle, "Fermix keeps stopping", Casing::Header),
    row(
        Key::ActivationCrashLoopBody,
        "The background service used up the starts it is allowed. Trying again clears that count and starts it once more.",
        Casing::Sentence,
    ),
    row(
        Key::ActivationProtocolMismatchTitle,
        "This window and the background service cannot talk",
        Casing::Header,
    ),
    row(
        Key::ActivationProtocolMismatchBody,
        "This window speaks versions {app_range} and the background service speaks {daemon_range}. Update both packages together.",
        Casing::Sentence,
    ),
    row(
        Key::ActivationBindFailureTitle,
        "Fermix could not take the address it needs",
        Casing::Header,
    ),
    row(
        Key::ActivationBindFailureBody,
        "Something else on this computer is already using the address Fermix listens on, most often a second account\u{2019}s Fermix. Choose another one and try again.",
        Casing::Sentence,
    ),
    row(
        Key::ActivationWebUnavailableTitle,
        "Fermix answers, but its web door does not",
        Casing::Header,
    ),
    row(
        Key::ActivationWebUnavailableBody,
        "The background service is running and the part of it your browser talks to is not answering yet.",
        Casing::Sentence,
    ),
    row(Key::ActivationLastLogLines, "The last few log lines", Casing::Header),
    // Version skew.
    row(Key::SkewNewerInstalledTitle, "A newer Fermix is installed", Casing::Sentence),
    row(
        Key::SkewNewerInstalledDetail,
        "The background service is still running the previous version, so your update has not taken effect yet. Your settings and conversations are unchanged.",
        Casing::Sentence,
    ),
    row(Key::SkewRestartToFinish, "Restart Fermix to finish updating", Casing::Header),
    row(
        Key::SkewUnknownIdentityTitle,
        "Fermix cannot tell which version is running",
        Casing::Sentence,
    ),
    row(
        Key::SkewUnknownIdentityBody,
        "The running service did not report a build it can be compared against, so nothing is claimed about whether it is up to date.",
        Casing::Sentence,
    ),
    row(Key::SkewStaleGuiTitle, "This window is older than the app installed", Casing::Sentence),
    row(
        Key::SkewStaleGuiBody,
        "Quit Fermix and open it again to use the version that is installed.",
        Casing::Sentence,
    ),
    row(
        Key::SkewOwnershipConflictTitle,
        "This Fermix is not the one this app manages",
        Casing::Sentence,
    ),
    row(
        Key::SkewOwnershipConflictBody,
        "What is installed and what is running disagree about who owns this account\u{2019}s Fermix, so nothing here will change it.",
        Casing::Sentence,
    ),
    // The Linux-only moments, M38 section 6.5, carried verbatim.
    row(
        Key::LingerDeniedTitle,
        "Fermix cannot run in the background yet",
        Casing::Header,
    ),
    row(
        Key::LingerDeniedBody,
        "Linux needs one more permission before Fermix can keep answering after you log out or restart. Nothing was installed and your settings are unchanged.",
        Casing::Sentence,
    ),
    row(
        Key::LingerDeniedCommandRow,
        "Run this once in a terminal: sudo loginctl enable-linger <user>",
        Casing::Sentence,
    ),
    row(
        Key::LingerDeniedCommandValue,
        "sudo loginctl enable-linger <user>",
        Casing::Sentence,
    ),
    row(Key::LoginManagerAbsentTitle, "This host has no login manager", Casing::Header),
    row(
        Key::LoginManagerAbsentBody,
        "Fermix could not find loginctl, so it cannot get permission to keep running after you log out. Nothing was installed and your settings are unchanged.",
        Casing::Sentence,
    ),
    row(
        Key::LoginManagerAbsentNextAction,
        "Install systemd\u{2019}s login manager on this host, then try again",
        Casing::Sentence,
    ),
    row(Key::SecretStoreDesktopKeyring, "Stored in your desktop keyring", Casing::Sentence),
    row(Key::SecretStorePasswordStore, "Stored in your password store", Casing::Sentence),
    row(Key::SecretStoreNone, "No store on this host", Casing::Sentence),
    row(
        Key::SecretStoreUnavailableTitle,
        "Fermix has nowhere to store this key",
        Casing::Header,
    ),
    row(
        Key::SecretStoreUnavailableBody,
        "This host has no unlocked keyring, so the key was not saved. Everything else you entered is unchanged.",
        Casing::Sentence,
    ),
    row(
        Key::SecretStoreUnavailableNextAction,
        "Install pass and pass-secret-service, or start a desktop keyring, then add the key again",
        Casing::Sentence,
    ),
    row(
        Key::SystemScopeRefusalTitle,
        "Fermix is installed as a system service",
        Casing::Header,
    ),
    row(
        Key::SystemScopeRefusalBody,
        "This app manages Fermix for your account only, so it made no changes. Found a unit at /etc/systemd/system/fermix.service running as root, and a Fermix home that belongs to you.",
        Casing::Sentence,
    ),
    row(
        Key::SystemScopeRefusalNextAction,
        "In a terminal run sudo fermix service uninstall --system, then fermix service install",
        Casing::Sentence,
    ),
    row(
        Key::SystemScopeRefusalCommandValue,
        "sudo fermix service uninstall --system\nfermix service install",
        Casing::Sentence,
    ),
    // The one notification this application raises.
    row(
        Key::NoticeAttentionTitle,
        "Something needs your attention",
        Casing::Header,
    ),
    row(
        Key::NoticeAttentionBody,
        "Open Fermix to see what changed.",
        Casing::Sentence,
    ),
];

/// Every key whose source carries a substitution marker, with the markers it
/// carries. A template that is not declared here fails `tests/copy.rs`, which
/// is what stops a marker from reaching a person unfilled.
pub const TEMPLATED: &[(Key, &[&str])] = &[
    (Key::RestartConversationsKnown, &["{count}"]),
    (Key::HomeUptimeDaysHours, &["{days}", "{hours}"]),
    (Key::HomeUptimeHoursMinutes, &["{hours}", "{minutes}"]),
    (Key::HomeUptimeMinutes, &["{minutes}"]),
    (Key::ProviderProbeResult, &["{count}", "{model}"]),
    (Key::IntegrationsSwitchAccessible, &["{name}"]),
    (Key::CountedFilter, &["{name}", "{count}"]),
    (Key::ComputerProbedAt, &["{time}"]),
    (Key::ComputerSessionDetected, &["{session}"]),
    (Key::DoctorSummaryFailed, &["{count}"]),
    (Key::LogsCaptionFile, &["{path}"]),
    (
        Key::ActivationProtocolMismatchBody,
        &["{app_range}", "{daemon_range}"],
    ),
    (Key::LingerDeniedCommandRow, &["<user>"]),
    (Key::LingerDeniedCommandValue, &["<user>"]),
    (Key::SetupProgressAccessible, &["{step}", "{total}"]),
];

const fn row(key: Key, source: &'static str, casing: Casing) -> Entry {
    Entry {
        key,
        source,
        casing,
    }
}

/// One catalogue row, by key.
///
/// Panics on [`Key::Count`], which is a count and not a key.
pub fn entry(key: Key) -> &'static Entry {
    &CATALOGUE[key as usize]
}

/// The rendered English text of one key.
pub fn text(key: Key) -> String {
    let entry = entry(key);
    let translated = gettext(entry.source);

    // The casing transform runs over the English source only: a translation
    // carries its own language's capitalization conventions and Header
    // Capitalization is an English rule (M38 section 6.7).
    if translated != entry.source {
        return translated;
    }

    match entry.casing {
        Casing::Header => header_capitalized(&translated),
        Casing::Sentence => translated,
    }
}

/// The rendered text of one key with its markers filled in.
///
/// A marker the caller does not supply stays in the string, which is visible in
/// a capture and caught by the template gate rather than shipped quietly.
pub fn fill(key: Key, values: &[(&str, &str)]) -> String {
    let mut rendered = text(key);
    for (marker, value) in values {
        rendered = rendered.replace(marker, value);
    }
    rendered
}

/// Header Capitalization, GNOME's rule: capitalize every word except articles,
/// conjunctions and prepositions of fewer than four letters, and always
/// capitalize the first and the last word. A word that already carries a
/// capital is left exactly as it is, so an acronym survives.
pub fn header_capitalized(source: &str) -> String {
    let words: Vec<&str> = source.split(' ').collect();
    let last = words.len().saturating_sub(1);

    words
        .iter()
        .enumerate()
        .map(|(index, word)| capitalize_word(word, index == 0 || index == last))
        .collect::<Vec<String>>()
        .join(" ")
}

/// The closed set of words Header Capitalization leaves lowercase: articles,
/// conjunctions and prepositions of fewer than four letters.
const MINOR_WORDS: &[&str] = &[
    "a", "an", "and", "as", "at", "but", "by", "for", "if", "in", "nor", "of", "off", "on", "or",
    "out", "per", "so", "the", "to", "up", "via", "vs", "yet",
];

fn capitalize_word(word: &str, always: bool) -> String {
    if word.chars().any(char::is_uppercase) {
        return word.to_string();
    }
    if !always && MINOR_WORDS.contains(&trimmed_word(word).as_str()) {
        return word.to_string();
    }

    let mut characters = word.chars();
    match characters.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
    }
}

/// A word without the punctuation that can sit around it, so "disk." and
/// "disk" answer the minor-word question the same way.
fn trimmed_word(word: &str) -> String {
    word.chars()
        .filter(|character| character.is_alphanumeric())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_capitalization_follows_the_gnome_rule() {
        assert_eq!(
            header_capitalized("reload settings from disk"),
            "Reload Settings From Disk"
        );
        assert_eq!(header_capitalized("back to Fermix"), "Back to Fermix");
        assert_eq!(header_capitalized("run doctor"), "Run Doctor");
        assert_eq!(header_capitalized("open at login"), "Open at Login");
    }

    #[test]
    fn an_acronym_keeps_its_capitals() {
        assert_eq!(header_capitalized("MCPs"), "MCPs");
        assert_eq!(
            header_capitalized("use the Claude Code sign-in"),
            "Use the Claude Code Sign-in"
        );
    }

    #[test]
    fn the_last_word_is_capitalized_even_when_it_is_minor() {
        assert_eq!(header_capitalized("what this is for"), "What This Is For");
    }

    #[test]
    fn fill_substitutes_every_marker() {
        assert_eq!(
            fill(Key::DoctorSummaryFailed, &[("{count}", "3")]),
            "3 failed"
        );
    }
}
