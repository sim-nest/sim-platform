package org.simnest.shell

import android.app.Activity
import android.content.ComponentCallbacks2
import android.content.Intent
import android.content.res.Configuration
import android.media.AudioManager
import android.net.Uri
import android.os.Bundle
import org.json.JSONArray
import org.json.JSONObject

/** Thin typed packet pump. Rust owns policy; Android objects never cross this boundary. */
class SimActivity : Activity() {
    private external fun nativeInstantiate(): Long
    private external fun nativeDestroy(handle: Long)
    private external fun nativeCall(handle: Long, function: Int, frame: ByteArray): ByteArray

    private var capsule = 0L

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        capsule = nativeInstantiate()
        continuityEvent("restart")
        if (state != null) continuityEvent("activity-recreation")
        lifecycle("created")
        intent?.let(::activate)
    }

    override fun onResume() {
        super.onResume()
        lifecycle("active")
    }

    override fun onPause() {
        continuityEvent("suspend")
        lifecycle("suspended")
        super.onPause()
    }

    override fun onStop() {
        continuityEvent("background-restriction")
        super.onStop()
    }

    override fun onConfigurationChanged(configuration: Configuration) {
        super.onConfigurationChanged(configuration)
        continuityEvent("rotation")
    }

    override fun onTrimMemory(level: Int) {
        super.onTrimMemory(level)
        if (level >= ComponentCallbacks2.TRIM_MEMORY_RUNNING_LOW) continuityEvent("memory-pressure")
    }

    override fun onDestroy() {
        if (capsule != 0L) {
            lifecycle("stopped")
            nativeDestroy(capsule)
            capsule = 0L
        }
        super.onDestroy()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        activate(intent)
    }

    private fun invoke(function: Int, input: JSONObject): JSONObject =
        JSONObject(nativeCall(capsule, function, input.toString().encodeToByteArray()).decodeToString())

    private fun lifecycle(state: String): JSONObject =
        invoke(LIFECYCLE, JSONObject().put("type", "lifecycle").put("state", state))

    private fun continuityEvent(event: String): JSONObject =
        invoke(CONTINUITY, JSONObject().put("type", "event").put("event", event))

    private fun activate(intent: Intent): JSONObject =
        invoke(
            ACTIVATION,
            JSONObject()
                .put("type", "activation")
                .put("action", intent.action ?: "android.intent.action.VIEW")
                .put("content", intent.data?.let { boundedContentRef(intent, it) }),
        )

    private fun boundedContentRef(intent: Intent, uri: Uri): JSONObject {
        val persistable = intent.flags and Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION != 0
        val readable = intent.flags and Intent.FLAG_GRANT_READ_URI_PERMISSION != 0
        if (persistable && readable) {
            contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        return JSONObject()
            .put("kind", "table")
            .put("mount", "android-content")
            .put("key", JSONArray().put(uri.toString().hashCode().toUInt().toString(16)))
    }

    fun permissionResult(name: String, granted: Boolean): JSONObject =
        invoke(
            ACTIVATION,
            JSONObject()
                .put("type", "permission")
                .put("permission", name)
                .put("granted", granted),
        )

    fun notification(channel: String, bytes: ByteArray): JSONObject =
        invoke(
            ACTIVATION,
            JSONObject()
                .put("type", "notification")
                .put("channel", channel)
                .put("payload", JSONArray(bytes.map { it.toUByte().toInt() })),
        )

    fun audioDevice(manager: AudioManager, id: Int, connected: Boolean): JSONObject {
        manager.getDevices(AudioManager.GET_DEVICES_ALL)
        return invoke(
            ACTIVATION,
            JSONObject()
                .put("type", "audio-device")
                .put("id", id.toString())
                .put("connected", connected),
        )
    }

    fun backgroundExecution(allowed: Boolean): JSONObject =
        invoke(
            ACTIVATION,
            JSONObject()
                .put("type", "background-execution")
                .put("allowed", allowed),
        )

    internal fun testLifecycle(state: String): JSONObject = lifecycle(state)

    internal fun testActivation(action: String): JSONObject =
        invoke(
            ACTIVATION,
            JSONObject()
                .put("type", "activation")
                .put("action", action)
                .put("content", JSONObject.NULL),
        )

    companion object {
        private const val LIFECYCLE = 0
        private const val ACTIVATION = 1
        private const val CONTINUITY = 2

        init {
            System.loadLibrary("sim_platform_android")
        }
    }
}
