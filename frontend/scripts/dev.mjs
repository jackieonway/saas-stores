import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { build, preview } from 'vite'
import { createStoresConfig, readPublicConfig } from '../vite.config.mjs'

// The first build validates the output owner and types before replacing dist.
execFileSync(process.execPath, [fileURLToPath(new URL('./build.mjs', import.meta.url))], { stdio: 'inherit', windowsHide: true })
const config = createStoresConfig(readPublicConfig())
const watcher = await build({ ...config, configFile: false, build: { ...config.build, watch: {} } })
let server
let closing = false
async function close() {
  if (closing) return
  closing = true
  await watcher.close()
  if (server) {
    server.httpServer.closeAllConnections()
    await new Promise(resolve => server.httpServer.close(resolve))
  }
}
try {
  server = await preview({ ...config, configFile: false })
  server.printUrls()
  console.log('门店 qiankun UMD 自动重建已开启；源码变更后刷新平台页面生效。')
  for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => { void close() })
} catch (error) {
  await close()
  throw error
}
