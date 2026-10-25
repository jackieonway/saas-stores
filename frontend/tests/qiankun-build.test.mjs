import assert from 'node:assert/strict'
import test from 'node:test'
import { spawn, spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import path from 'node:path'
import os from 'node:os'
import vm from 'node:vm'
import { fileURLToPath } from 'node:url'
import { physicalDirectory, requireReleaseVersion, verifyReleaseDirectory, verifyReleaseHtml, verifyUmdLifecycle } from '../deploy/verify-release.mjs'
import { installRelease } from '../deploy/local/install-release.mjs'

const packageVersion = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8')).version

test('qiankun release is a classic UMD lifecycle with complete relative CSS and no dynamic chunks', { timeout: 90000 }, async t => {
  const temporary = realpathSync(os.tmpdir()), root = realpathSync(mkdtempSync(path.join(temporary, 'platform-qiankun-stores-')))
  t.after(() => {
    assert.equal(path.dirname(realpathSync(root)), temporary)
    assert.ok(path.basename(root).startsWith('platform-qiankun-stores-'))
    rmSync(root, { recursive: true, force: true })
  })
  const output = path.join(root, 'release'), builder = fileURLToPath(new URL('../scripts/build.mjs', import.meta.url))
  const built = spawnSync(process.execPath, [builder, '--out-dir', output, '--allowed-host-origin', 'https://platform.example.com'], { encoding: 'utf8', windowsHide: true, timeout: 90000 })
  assert.equal(built.status, 0, built.stdout + built.stderr)
  assert.equal(verifyReleaseDirectory(output, packageVersion).runtime, 'qiankun')
  const info = JSON.parse(readFileSync(path.join(output, 'build-info.json'), 'utf8'))
  assert.equal(info.runtime, 'qiankun')
  assert.deepEqual(info.dynamicChunks, [])
  const html = readFileSync(path.join(output, 'index.html'), 'utf8')
  assert.match(html, /<div data-stores-mount>/)
  assert.match(html, /<script entry src="\.\/assets\/micro-app\.umd\.js"><\/script>/)
  assert.match(html, /<link rel="stylesheet" href="\.\/[^"\s]+\.css">/)
  assert.doesNotMatch(html, /type="module"|modulepreload|iframe-bridge|assets\/config/)
  const javascript = readFileSync(path.join(output, info.entry), 'utf8')
  assert.doesNotMatch(javascript, /\bimport\s*\(/)
  const sandbox = vm.createContext({ console, fetch, AbortController, DOMException, Headers, URL, TextEncoder, TextDecoder, ReadableStream, Response, Request, crypto, setTimeout, clearTimeout })
  new vm.Script(javascript, { filename: info.entry }).runInContext(sandbox)
  assert.deepEqual(Object.keys(sandbox.StoresMicroApp).sort(), ['bootstrap', 'mount', 'unmount', 'update'])
  for (const lifecycle of Object.values(sandbox.StoresMicroApp)) assert.equal(typeof lifecycle, 'function')
  // qiankun uses the sandbox's latest written property as a lifecycle fallback
  // when framework initialization changes import-html-entry's export probe.
  let latestSetProp
  const proxy = new Proxy({}, { set(target, property, value) { latestSetProp = property; target[property] = value; return true } })
  const strictSandbox = vm.createContext({ console, fetch, AbortController, DOMException, Headers, URL, TextEncoder, TextDecoder, ReadableStream, Response, Request, crypto, setTimeout, clearTimeout, window: proxy, self: proxy, globalThis: proxy })
  new vm.Script(javascript, { filename: info.entry }).runInContext(strictSandbox)
  assert.equal(latestSetProp, 'StoresMicroApp')
  assert.notEqual(proxy['stores-runtime-random-name'], proxy[latestSetProp])
  assert.deepEqual(Object.keys(proxy[latestSetProp]).sort(), ['bootstrap', 'mount', 'unmount', 'update'])
  assert.equal(proxy.StoresMicroApp.bootstrap.sdkContract.version, '3.0.0')
  assert.throws(() => proxy.StoresMicroApp.bootstrap.sdkContract.assertSupportedSdkVersion('1.0.0'), error => error.code === 'UNSUPPORTED_SDK_VERSION')

  const prefix = '/micro-proxy/stores/versions/test/'
  const server = createServer((request, response) => {
    const name = new URL(request.url, 'http://localhost').pathname.slice(prefix.length)
    if (!request.url.startsWith(prefix) || name.includes('..')) { response.writeHead(404); response.end(); return }
    try {
      response.setHeader('Content-Type', name.endsWith('.css') ? 'text/css' : name.endsWith('.html') ? 'text/html' : 'text/javascript')
      response.end(readFileSync(path.join(output, name)))
    } catch { response.writeHead(404); response.end() }
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  t.after(() => new Promise(resolve => { server.closeAllConnections(); server.close(resolve) }))
  const base = new URL(prefix, `http://127.0.0.1:${server.address().port}`)
  for (const [, resource] of html.matchAll(/(?:href|src)="(\.\/[^"#]+)"/g)) {
    const url = new URL(resource, base)
    assert.ok(url.pathname.startsWith(prefix))
    const response = await fetch(url)
    assert.equal(response.status, 200, resource)
    assert.equal(await response.text(), readFileSync(path.join(output, resource), 'utf8'))
  }

  const published = path.join(root, 'published'), empty = path.join(root, 'preexisting-empty'), concurrent = path.join(root, 'concurrent')
  for (const directory of [published, empty, concurrent]) mkdirSync(directory)
  installRelease(output, published, packageVersion)
  assert.equal(verifyReleaseDirectory(path.join(published, packageVersion), packageVersion, { published: true }).runtime, 'qiankun')
  assert.throws(() => installRelease(output, published, packageVersion), /EEXIST/)
  assert.throws(() => installRelease(path.join(published, packageVersion), empty, packageVersion), /unpublished build/)
  assert.equal(existsSync(path.join(empty, packageVersion)), false)
  mkdirSync(path.join(empty, packageVersion))
  assert.throws(() => installRelease(output, empty, packageVersion), /EEXIST/)
  assert.equal(existsSync(path.join(empty, packageVersion, 'build-info.json')), false)
  const installer = new URL('../deploy/local/install-release.mjs', import.meta.url).href
  const child = () => new Promise((resolve, reject) => {
    const script = `import { installRelease } from ${JSON.stringify(installer)}; installRelease(${JSON.stringify(output)},${JSON.stringify(concurrent)},${JSON.stringify(packageVersion)});`
    const childProcess = spawn(process.execPath, ['--input-type=module', '-e', script], { stdio: 'ignore', windowsHide: true })
    childProcess.once('error', reject); childProcess.once('exit', resolve)
  })
  assert.deepEqual((await Promise.all([child(), child()])).sort(), [0, 1])
  verifyReleaseDirectory(path.join(concurrent, packageVersion), packageVersion, { published: true })
  const alias = path.join(root, 'alias')
  symlinkSync(published, alias, process.platform === 'win32' ? 'junction' : 'dir')
  assert.throws(() => physicalDirectory(alias), /symbolic links or junctions/)
  assert.throws(() => verifyReleaseDirectory(path.join(alias, packageVersion), packageVersion), /symbolic links or junctions/)
  assert.throws(() => installRelease(output, alias, packageVersion), /symbolic links or junctions/)

  for (const [field, value] of [['runtime', 'iframe'], ['sdkVersion', '1.0.0'], ['contractVersion', '1.0'], ['version', 'wrong-version'], ['entry', '../escape.js'], ['dynamicChunks', ['assets/lazy.js']]]) {
    writeFileSync(path.join(output, 'build-info.json'), JSON.stringify({ ...info, [field]: value }))
    assert.throws(() => verifyReleaseDirectory(output, packageVersion), undefined, field)
  }
  writeFileSync(path.join(output, 'build-info.json'), JSON.stringify(info))
  writeFileSync(path.join(output, 'index.html'), html.replace('<script entry', '<script type="module" entry'))
  assert.throws(() => verifyReleaseDirectory(output, packageVersion), /Legacy/)
  writeFileSync(path.join(output, 'index.html'), html)
  writeFileSync(path.join(output, info.entry), 'window.StoresMicroApp = { bootstrap() {}, mount() {}, unmount() {} };')
  assert.throws(() => verifyReleaseDirectory(output, packageVersion))
})

test('release tools require an explicit version and reject retained iframe history', () => {
  for (const missing of [undefined, '', '../outside']) assert.throws(() => requireReleaseVersion(missing))
  for (const version of ['1.0.0', '1.0.1']) {
    const historical = fileURLToPath(new URL(`../../artifacts/micro-apps/stores/versions/${version}/`, import.meta.url))
    assert.throws(() => verifyReleaseDirectory(historical, version), /qiankun/)
  }
  assert.throws(() => verifyUmdLifecycle('window.StoresMicroApp = { bootstrap() {}, mount() {}, update: 1, unmount() {} };'), /functions/)
  assert.throws(() => verifyUmdLifecycle('window.StoresMicroApp = { bootstrap() {}, mount() {}, update() {}, unmount() {} };'), /SDK contract/)
  const shell = `const bootstrap = Object.defineProperty(function(){}, 'sdkContract', { value: Object.freeze({ version: '1.0.0', assertSupportedSdkVersion() {} }) }); window.StoresMicroApp = { bootstrap, mount(){}, update(){}, unmount(){} };`
  assert.throws(() => verifyUmdLifecycle(shell), /Actual bundled SDK/)
  assert.throws(() => verifyUmdLifecycle(shell.replace("version: '1.0.0'", "version: '3.0.0'")), /reject unsupported versions/)
  const environment = { ...process.env }
  delete environment.STORES_RELEASE_VERSION
  const gateway = spawnSync(process.execPath, ['deploy/local/verify-gateway.mjs'], { cwd: fileURLToPath(new URL('../', import.meta.url)), env: environment, encoding: 'utf8', windowsHide: true })
  assert.notEqual(gateway.status, 0)
  assert.match(gateway.stderr, /explicit release version/)
  const publish = spawnSync(process.execPath, ['deploy/local/publish-release.mjs', '--version', '1.0.1', '--allowed-host-origin', 'http://127.0.0.1:3000'], { cwd: fileURLToPath(new URL('../', import.meta.url)), encoding: 'utf8', windowsHide: true })
  assert.notEqual(publish.status, 0)
  assert.match(publish.stderr, /Immutable release already exists/)
})

test('HTML release gate follows parsed attributes, entity decoding and inert HTML', () => {
  const info = { entry: 'assets/micro-app.umd.js' }
  const html = attributes => `<div data-stores-mount></div><link rel=stylesheet href=./micro-app.css><script entry ${attributes} src="./assets/micro-app.umd.js"></script>`
  for (const type of [
    'type=module', 'TYPE=MODULE', 'type="m&#111;dule"', "type='&#x6d;odule'",
    'type=m&#111;dule', 'type=" &Tab;Module&NewLine; "',
    'type=module type=text/javascript',
  ]) assert.throws(() => verifyReleaseHtml(html(type), info), /non-classic/, type)
  assert.deepEqual(verifyReleaseHtml(html('TYPE="text&#x2f;javascript"'), info), ['assets/micro-app.umd.js', 'micro-app.css'])
  assert.deepEqual(verifyReleaseHtml(`<!-- <script type=module></script> -->${html('')}`, info), ['assets/micro-app.umd.js', 'micro-app.css'])
  assert.throws(() => verifyReleaseHtml(`<template>${html('')}</template>`, info), /mount root/)
  assert.throws(() => verifyReleaseHtml(html('').replace('rel=stylesheet', 'rel="moduleprel&#111;ad"')), /modulepreload/)
})
