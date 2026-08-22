import assert from "node:assert/strict";
import {BrowserWasmFrames, detectBrowserApis, performHostCall} from "./sim-browser-shell.mjs";

assert.deepEqual(detectBrowserApis({indexedDB: {}, fetch() {}, WebSocket: class {}}), {
  storage: true, fetch: true, websocket: true, clipboard: false,
  notification: false, permissions: false, wake_lock: false,
});

let freed = 0; const memory = new WebAssembly.Memory({initial: 1}); let cursor = 8;
const frames = new BrowserWasmFrames({memory,
  sim_browser_alloc(len) { const ptr = cursor; cursor += len; return ptr; },
  sim_browser_dealloc() { freed += 1; },
  sim_browser_named_call(_np, _nl, fp, fl) { const out = cursor; new Uint8Array(memory.buffer, out, fl).set(new Uint8Array(memory.buffer, fp, fl)); cursor += fl; return (BigInt(out) << 32n) | BigInt(fl); },
});
assert.deepEqual([...frames.call("platform/card", Uint8Array.of(1, 2, 3))], [1, 2, 3]);
assert.equal(freed, 3, "input and output buffers have one owner and are released");
await assert.rejects(() => performHostCall({kind: "render-dom"}, {}), /unsupported browser host operation/);
