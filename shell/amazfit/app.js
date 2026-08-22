import { BasePage } from '@zeppos/zml/base-page'

const MAX_FRAME_BYTES = 4096
const callbacks = new Set()

function boundedSend(ctx, frame) {
  const text = JSON.stringify(frame)
  if (text.length > MAX_FRAME_BYTES) throw new Error('proxy frame exceeds bound')
  return ctx.request({ method: 'proxyFrame', params: text })
}

Page(BasePage({
  state: { session: 0, sequence: 0 },
  onInit() { this.state.session = Date.now(); callbacks.add('lifecycle') },
  build() { callbacks.add('display'); boundedSend(this, { version: 1, session: this.state.session, sequence: this.state.sequence++, payload: { kind: 'acknowledgement', action: 'ready' } }) },
  onDestroy() { callbacks.clear() },
  onButton(event) { return boundedSend(this, { version: 1, session: this.state.session, sequence: this.state.sequence++, payload: { kind: 'button', key: String(event.key) } }) },
}))

// Deliberately no eval, Function constructor, timers that outlive lifecycle,
// ambient network access, or SIM behavior in this vendor shell.
