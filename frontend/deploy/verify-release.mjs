import assert from 'node:assert/strict'
import { lstatSync, readFileSync, realpathSync } from 'node:fs'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import vm from 'node:vm'
import { parse } from 'parse5'

export function requireReleaseVersion(value) {
  assert.equal(typeof value, 'string', 'An explicit release version is required')
  assert.ok(/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(value) && !value.includes('..'), 'Invalid explicit release version')
  return value
}

export function verifyReleaseMetadata(info, expectedVersion) {
  const version = requireReleaseVersion(expectedVersion)
  assert.equal(info?.application, 'stores', 'Release application must be stores')
  assert.equal(info.version, version, 'Release version does not match the selected immutable directory')
  assert.equal(info.runtime, 'qiankun', 'Release runtime must be qiankun')
  assert.equal(info.sdkVersion, '2.0.0', 'Release SDK must be 2.0.0')
  assert.equal(info.contractVersion, '2.0', 'Release context contract must be 2.0')
  assert.equal(info.entry, 'assets/micro-app.umd.js', 'Release entry must be the stores classic UMD')
  assert.deepEqual(info.dynamicChunks, [], 'Release must not contain dynamic chunks')
  return info
}

function relativeAsset(value) {
  assert.equal(typeof value, 'string', 'Release asset path must be a string')
  const normalized = value.startsWith('./') ? value.slice(2) : value
  assert.ok(/^[A-Za-z0-9][A-Za-z0-9._/-]*$/.test(normalized), 'Release asset path must remain relative')
  assert.ok(!normalized.split('/').some(part => !part || part === '.' || part === '..'), 'Release asset path must not escape its version')
  return normalized
}

export function verifyReleaseHtml(html, info) {
  const elements = []
  const visit = node => {
    if (node.tagName) elements.push(node)
    // Template contents are inert, so cannot satisfy a real entry/mount root.
    for (const child of node.childNodes || []) visit(child)
  }
  visit(parse(html))
  const attributes = node => new Map(node.attrs.map(({ name, value }) => [name, value]))
  assert.ok(elements.some(node => node.tagName === 'div' && attributes(node).has('data-stores-mount')), 'Release qiankun mount root is missing')
  assert.ok(!elements.some(node => node.tagName === 'iframe'), 'Legacy iframe release is not supported')
  const links = elements.filter(node => node.tagName === 'link')
  for (const node of links) assert.ok(!attributes(node).get('rel')?.toLowerCase().split(/\s+/).includes('modulepreload'), 'Legacy ESM modulepreload is not supported')
  const scripts = elements.filter(node => node.tagName === 'script')
  assert.equal(scripts.length, 1, 'Release must contain one classic entry script')
  assert.equal(scripts[0].namespaceURI, 'http://www.w3.org/1999/xhtml', 'Release entry must be an HTML script element')
  const entry = attributes(scripts[0])
  const type = (entry.get('type') || '').trim().toLowerCase()
  assert.ok(['', 'text/javascript', 'application/javascript'].includes(type), 'Legacy ESM or non-classic script type is not supported')
  assert.ok(entry.has('entry'), 'qiankun script must carry the entry attribute')
  assert.ok(!(scripts[0].childNodes || []).some(node => node.value?.trim()), 'Release must not contain inline entry code')
  const source = entry.get('src')
  assert.equal(source, `./${info.entry}`, 'HTML entry must match build-info')
  const css = links.filter(node => attributes(node).get('rel')?.toLowerCase().split(/\s+/).includes('stylesheet'))
    .map(node => relativeAsset(attributes(node).get('href')))
  assert.ok(css.length > 0, 'Release CSS is missing')
  for (const asset of css) assert.match(asset, /\.css$/, 'Stylesheet must reference a CSS asset')
  return [...new Set([relativeAsset(source), ...css])]
}

export function verifyUmdLifecycle(javascript) {
  assert.doesNotMatch(javascript, /\bimport\s*\(|iframe-bridge|MicroIframe/, 'Release must not load dynamic ESM or an iframe bridge')
  let latestSetProp
  const globals = new Proxy({}, { set(target, property, value) { latestSetProp = property; target[property] = value; return true } })
  // Validate our own compiled exports without calling lifecycle methods or
  // exposing Node/process/require, network access, timers or a real browser DOM.
  const denied = () => { throw new Error('Release validation cannot perform browser side effects') }
  const sandbox = vm.createContext({
    window: globals, self: globals, globalThis: globals,
    console: Object.freeze({ log() {}, warn() {}, error() {} }), fetch: denied,
    AbortController, DOMException, Headers, URL, TextEncoder, TextDecoder,
    ReadableStream, Response, Request, crypto, setTimeout: denied, clearTimeout() {},
  }, { codeGeneration: { strings: false, wasm: false } })
  new vm.Script(javascript, { filename: 'micro-app.umd.js' }).runInContext(sandbox, { timeout: 3000 })
  assert.equal(latestSetProp, 'StoresMicroApp', 'qiankun latest export must be StoresMicroApp')
  const lifecycle = globals.StoresMicroApp
  assert.ok(lifecycle && typeof lifecycle === 'object', 'UMD lifecycle export is missing')
  assert.deepEqual(Object.keys(lifecycle).sort(), ['bootstrap', 'mount', 'unmount', 'update'])
  for (const action of Object.values(lifecycle)) assert.equal(typeof action, 'function', 'qiankun lifecycle exports must be functions')
  const descriptor = Object.getOwnPropertyDescriptor(lifecycle.bootstrap, 'sdkContract')
  assert.ok(descriptor && !descriptor.writable && !descriptor.configurable, 'Bundled SDK contract must be a readonly bootstrap property')
  const contract = descriptor.value
  assert.ok(Object.isFrozen(contract), 'Bundled SDK contract must be frozen')
  assert.equal(contract.version, '2.0.0', 'Actual bundled SDK must be 2.0.0')
  assert.equal(typeof contract.assertSupportedSdkVersion, 'function', 'Bundled SDK version negotiation is required')
  assert.doesNotThrow(() => contract.assertSupportedSdkVersion('2.0.0'), 'Bundled SDK must support version 2')
  for (const rejected of ['1.0.0', '3.0.0', undefined]) {
    assert.throws(() => contract.assertSupportedSdkVersion(rejected), error => error?.code === 'UNSUPPORTED_SDK_VERSION', 'Bundled SDK must reject unsupported versions')
  }
}

export function physicalDirectory(directory) {
  for (let current = path.resolve(directory); ; current = path.dirname(current)) {
    assert.ok(!lstatSync(current).isSymbolicLink(), 'Release paths must not contain symbolic links or junctions')
    if (path.dirname(current) === current) break
  }
  assert.ok(lstatSync(directory).isDirectory(), 'Release root must be a directory')
  return realpathSync(directory)
}

export function verifyPublishedMarker(marker, info) {
  assert.equal(marker?.complete, true, 'Published release completion marker is required')
  assert.equal(marker.version, info.version, 'Published release marker version must match')
  assert.match(info.sourceSha256, /^[a-f0-9]{64}$/, 'Published release source digest is required')
  assert.equal(marker.sourceSha256, info.sourceSha256, 'Published release marker must match the source digest')
}

export function verifyReleaseDirectory(directory, expectedVersion, { published = false } = {}) {
  const root = physicalDirectory(directory)
  function read(name) {
    const file = realpathSync(path.join(root, relativeAsset(name)))
    const relative = path.relative(root, file)
    assert.ok(relative && !path.isAbsolute(relative) && relative !== '..' && !relative.startsWith(`..${path.sep}`), 'Release file must remain inside its selected directory')
    return readFileSync(file, 'utf8')
  }
  const info = verifyReleaseMetadata(JSON.parse(read('build-info.json')), expectedVersion)
  const assets = new Set(verifyReleaseHtml(read('index.html'), info))
  const manifest = JSON.parse(read('asset-manifest.json'))
  assert.ok(manifest && typeof manifest === 'object' && !Array.isArray(manifest), 'Asset manifest must be an object')
  let entries = 0
  for (const record of Object.values(manifest)) {
    assert.ok(record && typeof record === 'object', 'Asset manifest record must be an object')
    assets.add(relativeAsset(record.file))
    for (const css of record.css || []) assets.add(relativeAsset(css))
    if (record.isEntry) { entries++; assert.equal(record.file, info.entry, 'Manifest entry must match build-info') }
  }
  assert.equal(entries, 1, 'Asset manifest must contain one UMD entry')
  for (const asset of assets) assert.ok(read(asset).length > 0, 'Release asset must not be empty')
  for (const dependency of ['react', 'react-dom']) assert.ok(read(`assets/${dependency}.LICENSE.txt`).length > 0, 'Bundled dependency license is missing')
  verifyUmdLifecycle(read(info.entry))
  if (published) verifyPublishedMarker(JSON.parse(read('release-complete.json')), info)
  return { version: info.version, runtime: info.runtime, sdkVersion: info.sdkVersion, assets: [...assets] }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [directory, flag, version, mode, ...extra] = process.argv.slice(2)
  assert.ok(directory && flag === '--version' && (!mode || mode === '--published') && !extra.length, 'Usage: node verify-release.mjs <directory> --version <explicit-version> [--published]')
  const result = verifyReleaseDirectory(directory, requireReleaseVersion(version), { published: mode === '--published' })
  console.log(`Verified stores ${result.version}: ${result.runtime}, SDK ${result.sdkVersion}, ${result.assets.length} assets`)
}
