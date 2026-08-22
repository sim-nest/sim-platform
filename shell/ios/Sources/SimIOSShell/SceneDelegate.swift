import UIKit
import UserNotifications

/// Owns iOS scene/session and callback glue; SIM sees only named typed calls.
@MainActor public final class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    public var window: UIWindow?
    private var capsule: SimCapsule?

    public func scene(_ scene: UIScene, willConnectTo session: UISceneSession, options: UIScene.ConnectionOptions) {
        capsule = try? SimCapsule()
        _ = try? capsule?.lifecycle("connected")
        for context in options.urlContexts { _ = try? capsule?.activate(action: "open-url", url: context.url) }
    }
    public func sceneDidBecomeActive(_ scene: UIScene) { _ = try? capsule?.lifecycle("active") }
    public func sceneWillResignActive(_ scene: UIScene) { _ = try? capsule?.lifecycle("suspended") }
    public func sceneDidDisconnect(_ scene: UIScene) {
        _ = try? capsule?.lifecycle("disconnected")
        capsule = nil
    }
    public func scene(_ scene: UIScene, openURLContexts contexts: Set<UIOpenURLContext>) {
        for context in contexts { _ = try? capsule?.activate(action: "open-url", url: context.url) }
    }
}
