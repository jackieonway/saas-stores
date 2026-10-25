import assert from 'node:assert/strict'
import test from 'node:test'
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { readPublicConfig } from '../vite.config.mjs'

const appRoot = fileURLToPath(new URL('../', import.meta.url))
const builder = path.join(appRoot, 'scripts/build.mjs')
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex')
function temporary(t) {
  const parent = realpathSync(os.tmpdir()), root = realpathSync(mkdtempSync(path.join(parent, 'platform-stores-build-')))
  t.after(() => {
    assert.equal(path.dirname(root), parent)
    assert.ok(path.basename(root).startsWith('platform-stores-build-'))
    assert.equal(realpathSync(root), root)
    rmSync(root, { recursive: true, force: true })
  })
  return root
}
function runBuild(output, extra = {}) {
  return spawnSync(process.execPath, [builder, '--allowed-host-origin', 'https://platform.example.com', '--out-dir', output, ...(extra.args || [])],
    { encoding: 'utf8', windowsHide: true, timeout: 90000, env: { ...process.env, ...extra.env } })
}
function allFiles(root) {
  return readdirSync(root, { withFileTypes: true }).flatMap(item => item.isDirectory() ? allFiles(path.join(root, item.name)) : [path.join(root, item.name)])
}

test('public configuration requires explicit precise secure origins and a safe API base', () => {
  for (const [origin, api] of [
    ['http://platform.example.com', '/stores-api'],
    ['http://localhost.example.com', '/stores-api'],
    ['https://platform.example.com', 'http://api.example.com/stores-api'],
    ['https://platform.example.com/', '/stores-api'],
    ['https://platform.example.com', '//api.example.com/stores-api'],
    ['https://platform.example.com', '/\\api.example.com/stores-api'],
    ['https://platform.example.com', 'https://user:password@api.example.com/stores-api'],
    ['https://platform.example.com', '/stores-api?token=private'],
  ]) assert.throws(() => readPublicConfig({ origins: [origin], 'api-base': api }))
  for (const origin of ['https://platform.example.com', 'http://localhost:3000', 'http://127.0.0.1:3000', 'http://[::1]:3000']) {
    assert.deepEqual(readPublicConfig({ version: '1.0.0-test', origins: [origin, origin] }), { version: '1.0.0-test', allowedHostOrigins: [origin], apiBase: '/stores-api' })
  }
  assert.throws(() => readPublicConfig({ version: '../outside', origins: ['https://platform.example.com'] }))
})

test('real Vite release emits a relative UMD React bundle with exact source, lock and SDK provenance and no private environment', { timeout: 90000 }, t => {
  const root = temporary(t), output = path.join(root, 'release'), secret = 'phase27-private-sentinel-never-public'
  const result = runBuild(output, { env: { VITE_CLIENT_SECRET: secret, DATABASE_URL: secret, STORES_CLIENT_SECRET: secret, PLATFORM_SESSION: secret } })
  assert.equal(result.status, 0, result.stdout + result.stderr)
  const json = name => JSON.parse(readFileSync(path.join(output, name), 'utf8'))
  const info = json('build-info.json'), manifest = json('asset-manifest.json')
  assert.equal(info.application, 'stores')
  assert.equal(info.sdkVersion, '3.0.0')
  assert.equal(info.runtime, 'qiankun')
  assert.equal(info.contractVersion, '2.0')
  assert.equal(info.reactVersion, '19.3.0')
  assert.equal(info.reactDomVersion, '19.3.0')
  assert.equal(info.typescriptVersion, '5.9.3')
  assert.equal(info.viteVersion, '8.3.4')
  assert.equal(info.nodeVersion, '24.22.1')
  assert.equal(info.version, JSON.parse(readFileSync(path.join(appRoot, 'package.json'), 'utf8')).version)
  assert.equal(info.inputCount, info.inputs.length)
  assert.equal(info.sourceSha256, sha256(JSON.stringify(info.inputs)))
  for (const input of info.inputs) {
    const bytes = readFileSync(path.join(appRoot, input.path))
    assert.equal(input.bytes, bytes.length, input.path)
    assert.equal(input.sha256, sha256(bytes), input.path)
  }
  assert.ok(info.inputs.some(input => input.path === 'src/qiankun-entry.ts'))
  assert.ok(info.inputs.some(input => input.path === 'src/app.tsx'))
  assert.ok(info.inputs.some(input => input.path === 'vendor/micro-sdk/index.mjs'))
  assert.ok(info.inputs.some(input => input.path === 'vendor/micro-sdk/index.d.mts'))
  assert.equal(info.packageLockSha256, sha256(readFileSync(path.join(appRoot, 'package-lock.json'))))
  const entry = Object.values(manifest).find(record => record.isEntry)
  assert.equal(entry.file, info.entry)
  assert.deepEqual(info.dynamicChunks, [])
  const html = readFileSync(path.join(output, 'index.html'), 'utf8')
  assert.match(html, /<script entry src="\.\/assets\/micro-app\.umd\.js"><\/script>/)
  assert.doesNotMatch(html, /type="module"|assets\/config\.js/)
  assert.doesNotMatch(html, /(?:src|href)="\/|https?:\/\//)
  for (const record of Object.values(manifest)) {
    for (const asset of [record.file, ...(record.css || [])]) {
      assert.ok(!path.isAbsolute(asset) && !asset.includes('..'))
      assert.equal(existsSync(path.join(output, asset)), true)
    }
  }
  for (const file of allFiles(output)) {
    const bytes = readFileSync(file, 'utf8')
    assert.ok(!bytes.includes(secret), path.relative(output, file))
  }
  for (const dependency of ['react', 'react-dom']) assert.equal(readFileSync(path.join(output, `assets/${dependency}.LICENSE.txt`), 'utf8'), readFileSync(path.join(appRoot, `node_modules/${dependency}/LICENSE`), 'utf8'))

  writeFileSync(path.join(output, 'obsolete-owned-asset.txt'), 'previous owned release')
  const rebuilt = runBuild(output, { args: ['--version', '1.0.1-rebuild'] })
  assert.equal(rebuilt.status, 0, rebuilt.stdout + rebuilt.stderr)
  assert.equal(existsSync(path.join(output, 'obsolete-owned-asset.txt')), false)
  assert.equal(json('build-info.json').version, '1.0.1-rebuild')
})

test('foreign output, non-temporary custom output and junction ancestors are preserved', t => {
  const root = temporary(t), foreign = path.join(root, 'foreign')
  mkdirSync(foreign); writeFileSync(path.join(foreign, 'keep.txt'), 'foreign-output')
  const result = runBuild(foreign)
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /不属于本应用构建/)
  assert.equal(readFileSync(path.join(foreign, 'keep.txt'), 'utf8'), 'foreign-output')
  const outside = path.join(appRoot, 'must-not-be-created')
  assert.equal(existsSync(outside), false)
  const rejected = runBuild(outside)
  assert.notEqual(rejected.status, 0)
  assert.match(rejected.stderr, /自定义输出须位于系统临时目录内/)
  assert.equal(existsSync(outside), false)
  const link = path.join(root, 'redirect')
  symlinkSync(foreign, link, process.platform === 'win32' ? 'junction' : 'dir')
  const linked = runBuild(path.join(link, 'release'))
  assert.notEqual(linked.status, 0)
  assert.match(linked.stderr, /符号链接/)
  assert.equal(existsSync(path.join(foreign, 'release')), false)
})

test('a copied standalone frontend installs from an empty owned npm cache and builds outside repository node_modules', { timeout: 240000 }, t => {
  const root = temporary(t), copy = path.join(root, 'frontend'), cache = path.join(root, 'empty-npm-cache')
  cpSync(appRoot, copy, { recursive: true, filter: source => !['node_modules', 'dist', '.npm-cache'].includes(path.relative(appRoot, source).split(path.sep)[0]) })
  assert.equal(existsSync(path.join(copy, 'node_modules')), false)
  assert.equal(existsSync(cache), false)
  const candidates = [process.env.npm_execpath, path.join(path.dirname(process.execPath), 'node_modules/npm/bin/npm-cli.js'), path.resolve(path.dirname(process.execPath), '../lib/node_modules/npm/bin/npm-cli.js')].filter(Boolean)
  const npmCli = candidates.find(candidate => existsSync(candidate))
  assert.ok(npmCli, 'a concrete npm CLI paired with the fixed Node runtime is required')
  const env = { ...process.env, PATH: path.dirname(process.execPath) + path.delimiter + process.env.PATH, NODE_PATH: '', npm_config_cache: cache, npm_config_fetch_retries: '1', npm_config_update_notifier: 'false', STORES_ALLOWED_HOST_ORIGINS: 'https://platform.example.com' }
  const install = spawnSync(process.execPath, [npmCli, 'ci', '--cache', cache, '--no-audit', '--no-fund'], { cwd: copy, encoding: 'utf8', windowsHide: true, timeout: 180000, env })
  assert.equal(install.status, 0, install.stdout + install.stderr)
  const require = createRequire(path.join(copy, 'package.json'))
  for (const dependency of ['react', 'react-dom', 'typescript', 'vite']) assert.ok(realpathSync(require.resolve(dependency)).startsWith(path.join(realpathSync(copy), 'node_modules') + path.sep), dependency)
  const build = spawnSync(process.execPath, [npmCli, 'run', 'build'], { cwd: copy, encoding: 'utf8', windowsHide: true, timeout: 90000, env })
  assert.equal(build.status, 0, build.stdout + build.stderr)
  assert.equal(JSON.parse(readFileSync(path.join(copy, 'dist/build-info.json'), 'utf8')).sdkVersion, '3.0.0')
  assert.equal(existsSync(path.join(copy, 'dist/index.html')), true)
})
