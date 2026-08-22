import AppKit
import AVFoundation
import CoreAudio
import CoreMIDI
import Metal
import Network
import UserNotifications

// Permission observation and prompting deliberately have distinct entrypoints.
@_cdecl("sim_macos_permission_status")
public func permissionStatus(_ kind: Int32) -> Int32 {
    return Int32(AVCaptureDevice.authorizationStatus(for: kind == 0 ? .video : .audio).rawValue)
}

@_cdecl("sim_macos_permission_request")
public func permissionRequest(_ kind: Int32) -> Int32 {
    // Called only after Rust verifies platform/permission-request/<kind>.
    AVCaptureDevice.requestAccess(for: kind == 0 ? .video : .audio) { _ in }
    return 0
}
