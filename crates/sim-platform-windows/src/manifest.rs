//! Authored portable-service to Windows-API map.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceBinding {
    pub service: &'static str,
    pub api: &'static str,
    pub permission: &'static str,
}

/// Complete capsule manifest. API names never become dispatch identities.
pub const SERVICES: &[ServiceBinding] = &[
    ServiceBinding {
        service: "filesystem",
        api: "CreateFileW, GetFinalPathNameByHandleW",
        permission: "broadFileSystemAccess or preopened picker token",
    },
    ServiceBinding {
        service: "process",
        api: "CreateProcessW, job objects, CancelSynchronousIo",
        permission: "runFullTrust",
    },
    ServiceBinding {
        service: "loader",
        api: "LoadPackagedLibrary, GetProcAddress",
        permission: "package graph",
    },
    ServiceBinding {
        service: "socket",
        api: "Winsock2, Windows.Networking.Sockets",
        permission: "internetClient, privateNetworkClientServer",
    },
    ServiceBinding {
        service: "lifecycle",
        api: "CoreApplication, AppLifecycle",
        permission: "none",
    },
    ServiceBinding {
        service: "activation",
        api: "AppInstance, IApplicationActivationManager",
        permission: "package identity",
    },
    ServiceBinding {
        service: "permission-status",
        api: "AppCapability.CheckAccess",
        permission: "none",
    },
    ServiceBinding {
        service: "permission-request",
        api: "AppCapability.RequestAccess",
        permission: "matching SIM capability",
    },
    ServiceBinding {
        service: "clipboard",
        api: "Windows.ApplicationModel.DataTransfer.Clipboard",
        permission: "interactiveWindow",
    },
    ServiceBinding {
        service: "notification",
        api: "AppNotificationManager",
        permission: "package identity",
    },
    ServiceBinding {
        service: "audio",
        api: "WASAPI, Media Foundation",
        permission: "microphone when capture is requested",
    },
    ServiceBinding {
        service: "midi",
        api: "Windows.Devices.Midi, WinMM",
        permission: "midi",
    },
    ServiceBinding {
        service: "compute",
        api: "D3D12, DXGI, DirectML",
        permission: "none",
    },
];

#[must_use]
pub fn generated_service_set() -> String {
    SERVICES
        .iter()
        .map(|binding| binding.service)
        .collect::<Vec<_>>()
        .join(",")
}
