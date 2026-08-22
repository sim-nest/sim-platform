//! Authored service-to-framework map. This is defined before implementation.

/// Prompt behavior of a native API binding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptPolicy {
    /// The binding is incapable of displaying a permission prompt.
    Never,
    /// The binding may prompt only through `platform/permission-request`.
    PermissionRequestOnly,
}

/// One frozen portable service realized by private macOS mechanics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceBinding {
    pub service: &'static str,
    pub framework: &'static str,
    pub api: &'static str,
    pub prompt: PromptPolicy,
}

/// Complete macOS service manifest. Framework names are implementation facts,
/// never portable dispatch identities.
pub const SERVICES: &[ServiceBinding] = &[
    ServiceBinding {
        service: "filesystem",
        framework: "Foundation",
        api: "NSFileCoordinator, file descriptors",
        prompt: PromptPolicy::Never,
    },
    ServiceBinding {
        service: "process",
        framework: "Foundation",
        api: "NSTask, posix_spawn",
        prompt: PromptPolicy::Never,
    },
    ServiceBinding {
        service: "loader",
        framework: "Darwin",
        api: "dlopen/dlsym",
        prompt: PromptPolicy::Never,
    },
    ServiceBinding {
        service: "socket",
        framework: "Network",
        api: "Network.framework, BSD sockets",
        prompt: PromptPolicy::Never,
    },
    ServiceBinding {
        service: "lifecycle",
        framework: "AppKit",
        api: "NSApplicationDelegate",
        prompt: PromptPolicy::Never,
    },
    ServiceBinding {
        service: "activation",
        framework: "AppKit",
        api: "NSRunningApplication.activate",
        prompt: PromptPolicy::Never,
    },
    ServiceBinding {
        service: "clipboard",
        framework: "AppKit",
        api: "NSPasteboard",
        prompt: PromptPolicy::Never,
    },
    ServiceBinding {
        service: "notification",
        framework: "UserNotifications",
        api: "UNUserNotificationCenter",
        prompt: PromptPolicy::Never,
    },
    ServiceBinding {
        service: "permission-status",
        framework: "AVFoundation/CoreGraphics",
        api: "authorizationStatus/preflightAccess",
        prompt: PromptPolicy::Never,
    },
    ServiceBinding {
        service: "permission-request",
        framework: "AVFoundation/CoreGraphics",
        api: "requestAccess/requestScreenCaptureAccess",
        prompt: PromptPolicy::PermissionRequestOnly,
    },
    ServiceBinding {
        service: "audio",
        framework: "CoreAudio",
        api: "AudioObject/AudioUnit",
        prompt: PromptPolicy::Never,
    },
    ServiceBinding {
        service: "midi",
        framework: "CoreMIDI",
        api: "MIDIClient/MIDIPort",
        prompt: PromptPolicy::Never,
    },
    ServiceBinding {
        service: "compute",
        framework: "Metal",
        api: "MTLCreateSystemDefaultDevice",
        prompt: PromptPolicy::Never,
    },
    ServiceBinding {
        service: "open",
        framework: "AppKit",
        api: "NSWorkspace.open",
        prompt: PromptPolicy::Never,
    },
    ServiceBinding {
        service: "share",
        framework: "AppKit",
        api: "NSSharingService",
        prompt: PromptPolicy::Never,
    },
];
