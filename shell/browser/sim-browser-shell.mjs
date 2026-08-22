// Browser host membrane: API detection, host calls, and byte movement only.
export function detectBrowserApis(host = globalThis) {
  return {
    storage: Boolean(host.indexedDB), fetch: typeof host.fetch === "function",
    websocket: typeof host.WebSocket === "function",
    clipboard: Boolean(host.navigator?.clipboard),
    notification: typeof host.Notification === "function",
    permissions: Boolean(host.navigator?.permissions),
    wake_lock: Boolean(host.navigator?.wakeLock),
  };
}

export class BrowserWasmFrames {
  constructor(exports) { this.exports = exports; }
  call(name, frame) {
    const nameBytes = new TextEncoder().encode(name);
    const namePtr = this.#copyIn(nameBytes); const framePtr = this.#copyIn(frame);
    try {
      const packed = BigInt(this.exports.sim_browser_named_call(namePtr, nameBytes.length, framePtr, frame.length));
      if (packed === 0n) throw new Error("browser capsule rejected named call");
      const ptr = Number(packed >> 32n), len = Number(packed & 0xffffffffn);
      const result = new Uint8Array(this.exports.memory.buffer, ptr, len).slice();
      this.exports.sim_browser_dealloc(ptr, len); return result;
    } finally {
      this.exports.sim_browser_dealloc(namePtr, nameBytes.length);
      this.exports.sim_browser_dealloc(framePtr, frame.length);
    }
  }
  #copyIn(bytes) {
    const ptr = this.exports.sim_browser_alloc(bytes.length);
    new Uint8Array(this.exports.memory.buffer, ptr, bytes.length).set(bytes); return ptr;
  }
}

// This table is deliberately mechanical: Rust chooses an operation and the
// shell performs exactly that operation. No SIM routing or presentation policy lives here.
export async function performHostCall(operation, host = globalThis) {
  switch (operation.kind) {
    case "clipboard-read": return new TextEncoder().encode(await host.navigator.clipboard.readText());
    case "clipboard-write": await host.navigator.clipboard.writeText(operation.text); return new Uint8Array();
    case "permission": return new TextEncoder().encode((await host.navigator.permissions.query({name: operation.name})).state);
    case "notification": new host.Notification(operation.title, {body: operation.body}); return new Uint8Array();
    case "wake-lock": return operation.acquire ? host.navigator.wakeLock.request("screen") : new Uint8Array();
    case "fetch": return new Uint8Array(await (await host.fetch(operation.url, {method: operation.method, body: operation.body})).arrayBuffer());
    default: throw new Error(`unsupported browser host operation: ${operation.kind}`);
  }
}
