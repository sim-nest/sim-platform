import { BaseSideService } from '@zeppos/zml/base-side'

AppSideService(BaseSideService({
  onInit() {},
  onRequest(request, response) {
    if (request.method !== 'proxyFrame' || typeof request.params !== 'string' || request.params.length > 4096) return response(null, { code: 'invalid-frame' })
    return response({ accepted: true })
  },
  onDestroy() {},
}))
