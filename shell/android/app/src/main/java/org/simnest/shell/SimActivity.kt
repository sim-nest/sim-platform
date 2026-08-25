package org.simnest.shell

import android.app.Activity
import android.content.ComponentCallbacks2
import android.content.Intent
import android.content.res.Configuration
import android.media.AudioManager
import android.media.AudioDeviceInfo
import android.media.AudioDeviceCallback
import android.media.AudioFocusRequest
import android.media.AudioAttributes
import android.os.Build
import android.os.SystemClock
import android.net.Uri
import android.os.Bundle
import android.speech.SpeechRecognizer
import android.speech.tts.Voice
import org.json.JSONArray
import org.json.JSONObject

/** Thin typed packet pump. Rust owns policy; Android objects never cross this boundary. */
class SimActivity : Activity() {
    private external fun nativeInstantiate(): Long
    private external fun nativeDestroy(handle: Long)
    private external fun nativeCall(handle: Long, function: Int, frame: ByteArray): ByteArray

    private var capsule = 0L
    private lateinit var audio: AndroidAudioCallbacks

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        capsule = nativeInstantiate()
        audio = AndroidAudioCallbacks(getSystemService(AudioManager::class.java), ::androidAudio)
        audio.start()
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
        audio.release("cancellation")
        continuityEvent("suspend")
        lifecycle("suspended")
        super.onPause()
    }

    override fun onStop() {
        audio.release("background-expiry")
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
            audio.close()
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

    fun permissionResult(name: String, granted: Boolean): JSONObject {
        if (name == "microphone" && !granted) audio.release("permission-loss")
        return invoke(
            ACTIVATION,
            JSONObject()
                .put("type", "permission")
                .put("permission", name)
                .put("granted", granted),
        )
    }

    fun notification(channel: String, bytes: ByteArray): JSONObject =
        invoke(
            ACTIVATION,
            JSONObject()
                .put("type", "notification")
                .put("channel", channel)
                .put("payload", JSONArray(bytes.map { it.toUByte().toInt() })),
        )

    private fun androidAudio(input: JSONObject): JSONObject =
        invoke(AUDIO, JSONObject().put("type", "audio").put("input", input))

    internal fun testArmAudio(turn: String, privateOutput: Boolean): JSONObject =
        audio.arm(turn, privateOutput)

    internal fun testStopAudio(reason: String): JSONObject = audio.release(reason)

    fun backgroundExecution(allowed: Boolean): JSONObject =
        invoke(
            ACTIVATION,
            JSONObject()
                .put("type", "background-execution")
                .put("allowed", allowed),
        )

    internal fun testLifecycle(state: String): JSONObject = lifecycle(state)

    /** Discovery is observational: it never constructs the remote-capable system recognizer. */
    internal fun discoverOnDeviceRecognizer(): Boolean =
        Build.VERSION.SDK_INT >= 31 && SpeechRecognizer.isOnDeviceRecognitionAvailable(this)

    /** Called only by a network-denied installation test after discovery succeeds. */
    internal fun createProvenOnDeviceRecognizer(): SpeechRecognizer? =
        if (discoverOnDeviceRecognizer()) SpeechRecognizer.createOnDeviceSpeechRecognizer(this) else null

    /** Filters an already-observed engine inventory; never invokes install or check-data activities. */
    internal fun installedOnDeviceTtsLanguages(voices: Set<Voice>): Set<String> =
        voices
            .asSequence()
            .filterNot(Voice::isNetworkConnectionRequired)
            .map { it.locale.toLanguageTag() }
            .filter(String::isNotBlank)
            .toSortedSet()

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
        private const val AUDIO = 3

        init {
            System.loadLibrary("sim_platform_android")
        }
    }
}

/** Android-only mechanics. Reports classes, never device identity or pairing state. */
internal class AndroidAudioCallbacks(
    private val manager: AudioManager,
    private val send: (JSONObject) -> JSONObject,
) : AudioDeviceCallback(), AudioManager.OnAudioFocusChangeListener {
    private var generation = 0L
    private var armed = false
    private var focusRequest: AudioFocusRequest? = null

    fun start() = manager.registerAudioDeviceCallback(this, null)

    fun arm(turn: String, privateOutput: Boolean): JSONObject {
        val request = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_EXCLUSIVE)
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                    .build(),
            )
            .setOnAudioFocusChangeListener(this)
            .build()
        if (manager.requestAudioFocus(request) != AudioManager.AUDIOFOCUS_REQUEST_GRANTED) {
            return send(JSONObject().put("action", "stop").put("reason", "focus-conflict"))
        }
        focusRequest = request
        manager.mode = AudioManager.MODE_IN_COMMUNICATION
        // API 31+ retains Android's current user-selected communication device; it never ranks one.
        if (Build.VERSION.SDK_INT >= 31) manager.communicationDevice?.let(manager::setCommunicationDevice)
        armed = true
        val now = SystemClock.elapsedRealtime()
        send(
            JSONObject()
                .put("action", "arm")
                .put("spec", JSONObject()
                    .put("turn_content_id", turn)
                    .put("api_level", Build.VERSION.SDK_INT)
                    .put("admitted", true)
                    .put("private_output", privateOutput)
                    .put("pcm", JSONObject()
                        .put("sample_rate_hz", 48_000)
                        .put("channels", 2)
                        .put("frames_per_chunk", 480)
                        .put("queue_capacity_chunks", 8))
                    .put("armed_at_ms", now)
                    .put("expires_at_ms", now + 30_000)),
        )
        return observe("initial-query")
    }

    fun release(reason: String): JSONObject {
        if (armed) {
            focusRequest?.let(manager::abandonAudioFocusRequest)
            if (Build.VERSION.SDK_INT >= 31) manager.clearCommunicationDevice()
            manager.mode = AudioManager.MODE_NORMAL
        }
        armed = false
        focusRequest = null
        return send(JSONObject().put("action", "stop").put("reason", reason))
    }

    fun close() {
        release("process-death")
        manager.unregisterAudioDeviceCallback(this)
    }

    override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>) {
        if (armed) observe("device-callback")
    }

    override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>) {
        if (armed) observe("device-callback")
    }

    override fun onAudioFocusChange(change: Int) {
        if (armed && change != AudioManager.AUDIOFOCUS_GAIN) release("focus-conflict")
    }

    private fun observe(evidence: String): JSONObject {
        generation += 1
        val capture = manager.getDevices(AudioManager.GET_DEVICES_INPUTS).mapNotNull(::routeClass).distinct().sorted()
        val render = manager.getDevices(AudioManager.GET_DEVICES_OUTPUTS).mapNotNull(::routeClass).distinct().sorted()
        return send(
            JSONObject()
                .put("action", "route")
                .put("observation", JSONObject()
                    .put("capture", JSONArray(capture))
                    .put("render", JSONArray(render))
                    .put("generation", generation)
                    .put("observed_at_ms", SystemClock.elapsedRealtime())
                    .put("evidence", evidence)),
        )
    }

    private fun routeClass(device: AudioDeviceInfo): String? = when (device.type) {
        AudioDeviceInfo.TYPE_BUILTIN_EARPIECE,
        AudioDeviceInfo.TYPE_BUILTIN_MIC,
        AudioDeviceInfo.TYPE_BUILTIN_SPEAKER -> "handset"
        AudioDeviceInfo.TYPE_WIRED_HEADSET,
        AudioDeviceInfo.TYPE_WIRED_HEADPHONES -> "wired"
        AudioDeviceInfo.TYPE_BLUETOOTH_SCO,
        AudioDeviceInfo.TYPE_BLUETOOTH_A2DP -> "classic-bluetooth"
        AudioDeviceInfo.TYPE_BLE_HEADSET,
        AudioDeviceInfo.TYPE_BLE_SPEAKER,
        AudioDeviceInfo.TYPE_BLE_BROADCAST -> "le-audio"
        AudioDeviceInfo.TYPE_USB_DEVICE,
        AudioDeviceInfo.TYPE_USB_HEADSET,
        AudioDeviceInfo.TYPE_USB_ACCESSORY -> "usb"
        AudioDeviceInfo.TYPE_UNKNOWN -> null
        else -> "other"
    }
}
