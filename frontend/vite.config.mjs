import { createHash } from 'node:crypto'
import { readFileSync, readdirSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { defineConfig, version as viteVersion } from 'vite'
import { SDK_VERSION } from './vendor/micro-sdk/index.mjs'

const appRoot = fileURLToPath(new URL('.', import.meta.url))
const sha256 = value => createHash('sha256').update(value).digest('hex')
export const OUTPUT_OWNER = 'stores-frontend-v1'

function requireSecureRemote(url) {
  if (url.protocol === 'http:' && !['localhost', '127.0.0.1', '[::1]'].includes(url.hostname)) {
    throw new Error('远程宿主与业务 API 必须使用 HTTPS；HTTP 仅允许 localhost、127.0.0.1 或 [::1] 回环地址')
  }
}
export function readPublicConfig(options = {}) {
  const version = options.version || process.env.STORES_VERSION || '1.0.0'
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(version) || version.includes('..')) throw new Error('版本号无效')
  const origins = options.origins?.length ? options.origins : (process.env.STORES_ALLOWED_HOST_ORIGINS || '').split(',').filter(Boolean)
  if (!origins.length) throw new Error('请显式设置 STORES_ALLOWED_HOST_ORIGINS 或 --allowed-host-origin；构建不会猜测宿主来源')
  for (const origin of origins) {
    const url = new URL(origin)
    if (!['http:', 'https:'].includes(url.protocol) || url.origin !== origin) throw new Error('宿主白名单必须是精确 HTTP(S) origin')
    requireSecureRemote(url)
  }
  const apiBase = options['api-base'] || process.env.STORES_API_BASE || '/stores-api'
  const apiUrl = new URL(apiBase, 'https://gateway.invalid')
  if (!/^(?:\/(?!\/)|https?:\/\/)/.test(apiBase) || /[\\\s]/.test(apiBase) || !['http:', 'https:'].includes(apiUrl.protocol) || apiUrl.username || apiUrl.password || apiUrl.search || apiUrl.hash || apiBase.startsWith('//')) throw new Error('业务 API 地址无效')
  requireSecureRemote(apiUrl)
  return Object.freeze({ version, allowedHostOrigins: [...new Set(origins)], apiBase })
}

function sourceInputs() {
  const files = ['index.html', 'package.json', 'package-lock.json', '.npmrc', 'tsconfig.json', 'vite.config.mjs']
  function visit(relative) {
    for (const entry of readdirSync(path.join(appRoot, relative), { withFileTypes: true })) {
      const name = `${relative}/${entry.name}`
      if (entry.isSymbolicLink()) throw new Error('构建源码不得使用符号链接')
      if (entry.isDirectory()) visit(name)
      else if (entry.isFile()) files.push(name)
    }
  }
  visit('src'); visit('scripts'); visit('vendor')
  return files.sort().map(file => {
    const bytes = readFileSync(path.join(appRoot, file))
    return { path: file, bytes: bytes.length, sha256: sha256(bytes) }
  })
}

export function createStoresConfig(publicConfig) {
  const javascriptAssets = { 'assets/config.js': `window.__STORES_CONFIG__ = Object.freeze(${JSON.stringify(publicConfig)});\n` }
  return {
    root: appRoot,
    base: './',
    appType: 'mpa',
    publicDir: false,
    envPrefix: [],
    plugins: [{
      name: 'stores-static-delivery',
      transformIndexHtml: {
        order: 'post',
        handler: () => Object.keys(javascriptAssets).map(src => ({ tag: 'script', attrs: { src: `./${src}`, crossorigin: 'anonymous' }, injectTo: 'head-prepend' })),
      },
      configureServer(server) {
        server.middlewares.use((request, response, next) => {
          const name = request.url?.split('?')[0]?.replace(/^\//, '')
          if (!Object.hasOwn(javascriptAssets, name)) return next()
          response.setHeader('Content-Type', 'text/javascript; charset=utf-8')
          response.end(javascriptAssets[name])
        })
      },
      generateBundle(_options, bundle) {
        for (const [fileName, source] of Object.entries(javascriptAssets)) this.emitFile({ type: 'asset', fileName, source })
        for (const dependency of ['react', 'react-dom']) this.emitFile({ type: 'asset', fileName: `assets/${dependency}.LICENSE.txt`, source: readFileSync(path.join(appRoot, 'node_modules', dependency, 'LICENSE'), 'utf8') })
        const inputs = sourceInputs()
        const entry = Object.values(bundle).find(item => item.type === 'chunk' && item.isEntry)
        const dynamicChunks = Object.values(bundle).filter(item => item.type === 'chunk' && item.isDynamicEntry).map(item => item.fileName).sort()
        const dependencyVersion = name => JSON.parse(readFileSync(path.join(appRoot, 'node_modules', name, 'package.json'), 'utf8')).version
        this.emitFile({ type: 'asset', fileName: 'build-info.json', source: JSON.stringify({
          application: 'stores', outputOwner: OUTPUT_OWNER, version: publicConfig.version,
          contractVersion: '2.0', sdkVersion: SDK_VERSION,
          reactVersion: dependencyVersion('react'), reactDomVersion: dependencyVersion('react-dom'),
          typescriptVersion: dependencyVersion('typescript'), viteVersion, nodeVersion: process.versions.node,
          sourceSha256: sha256(JSON.stringify(inputs)), inputCount: inputs.length,
          packageLockSha256: inputs.find(item => item.path === 'package-lock.json').sha256,
          inputs, entry: entry?.fileName, dynamicChunks, buildManifest: 'asset-manifest.json',
        }, null, 2) + '\n' })
      },
    }],
    server: { host: '127.0.0.1', port: 3014, strictPort: true, cors: { origin: '*' } },
    preview: { host: '127.0.0.1', port: 3014, strictPort: true, cors: { origin: '*' } },
    build: { outDir: 'dist', emptyOutDir: false, target: 'es2022', manifest: 'asset-manifest.json', sourcemap: false },
  }
}

export default defineConfig(() => createStoresConfig(readPublicConfig()))
