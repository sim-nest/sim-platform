package org.simnest.shell

import android.app.Activity
import android.content.Intent
import android.media.AudioManager
import android.net.Uri
import android.os.Bundle

/** Thin packet pump. Rust owns policy; Android objects never cross this boundary. */
class SimActivity : Activity() {
    private external fun nativeCall(function: String, frame: ByteArray): ByteArray
    override fun onCreate(state: Bundle?) { super.onCreate(state); lifecycle("created") ; intent?.let(::activate) }
    override fun onResume() { super.onResume(); lifecycle("active") }
    override fun onPause() { lifecycle("suspended"); super.onPause() }
    override fun onDestroy() { lifecycle("stopped"); super.onDestroy() }
    override fun onNewIntent(intent: Intent) { super.onNewIntent(intent); activate(intent) }
    private fun lifecycle(state: String) = nativeCall("platform/lifecycle", "{\"type\":\"lifecycle\",\"state\":\"$state\"}".encodeToByteArray())
    private fun activate(intent: Intent) { val ref = intent.data?.let(::boundedContentRef) ?: "null"; nativeCall("platform/activation", "{\"type\":\"activation\",\"action\":\"${intent.action ?: "view"}\",\"content\":$ref}".encodeToByteArray()) }
    private fun boundedContentRef(uri: Uri): String { contentResolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION); return "{\"kind\":\"table\",\"mount\":\"android-content\",\"key\":[\"${uri.toString().hashCode()}\"]}" }
    fun permissionResult(name: String, granted: Boolean) = nativeCall("platform/activation", "{\"type\":\"permission\",\"permission\":\"$name\",\"granted\":$granted}".encodeToByteArray())
    fun notification(channel: String, bytes: ByteArray) = nativeCall("platform/activation", "{\"type\":\"notification\",\"channel\":\"$channel\",\"payload\":[]}".encodeToByteArray())
    fun audioDevice(manager: AudioManager, id: Int, connected: Boolean) = nativeCall("platform/activation", "{\"type\":\"audio-device\",\"id\":\"$id\",\"connected\":$connected}".encodeToByteArray())
    fun backgroundExecution(allowed: Boolean) = nativeCall("platform/activation", "{\"type\":\"background-execution\",\"allowed\":$allowed}".encodeToByteArray())
    companion object { init { System.loadLibrary("sim_platform_android") } }
}
