import SimIOSShell
import XCTest

@MainActor final class SimCapsuleLifecycleTests: XCTestCase {
    func testSuspensionDenialPressureAndCleanupThroughStaticABI() throws {
        let capsule = try SimCapsule()
        _ = try capsule.lifecycle("connected")
        _ = try capsule.lifecycle("active")
        _ = try capsule.permission("microphone", granted: false)
        _ = try capsule.memoryPressure()
        _ = try capsule.lifecycle("suspended")
        XCTAssertNoThrow(try capsule.backgroundExecution(true))
        capsule.releaseAllDocumentGrants()
        _ = try capsule.lifecycle("disconnected")
    }
}
