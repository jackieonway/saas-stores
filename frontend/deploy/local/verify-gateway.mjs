// Read-only deployment checks. Run after the API, Vite and independent gateway start.
// Overrides: STORES_GATEWAY_ORIGIN, PLATFORM_DEV_ORIGIN, STORES_RELEASE_VERSION,
// STORES_GATEWAY_TIMEOUT_MS. No credentials, database access or browser state needed.
import assert from 'node:assert/strict'

function origin(value, label) {
  let parsed
  try { parsed = new URL(value) } catch { throw new Error(`${label} must be an exact HTTP(S) origin`) }
  if (!['http:', 'https:'].includes(parsed.protocol) || parsed.username || parsed.password || value !== parsed.origin) {
    throw new Error(`${label} must be an exact HTTP(S) origin without credentials or a trailing slash`)
  }
  return parsed.origin
}

const gateway = origin(process.env.STORES_GATEWAY_ORIGIN || 'http://127.0.0.1:3000', 'STORES_GATEWAY_ORIGIN')
const platform = origin(process.env.PLATFORM_DEV_ORIGIN || 'http://127.0.0.1:3002', 'PLATFORM_DEV_ORIGIN')
const version = process.env.STORES_RELEASE_VERSION || '1.0.1'
if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(version) || version.includes('..')) throw new Error('Invalid STORES_RELEASE_VERSION')
const timeout = Number(process.env.STORES_GATEWAY_TIMEOUT_MS || 10000)
if (!Number.isInteger(timeout) || timeout < 100 || timeout > 60000) throw new Error('Invalid STORES_GATEWAY_TIMEOUT_MS')
const releasePath = `/sub-apps/stores/versions/${version}/`
const indexPath = `${releasePath}index.html`
const checks = []

async function response(base, path, options = {}) {
  // Never accept a server redirect, which could send even read-only requests elsewhere.
  return fetch(new URL(path, base), {
    ...options, credentials: 'omit', redirect: 'error', signal: AbortSignal.timeout(timeout),
  })
}
function type(result) { return (result.headers.get('content-type') || '').split(';', 1)[0].toLowerCase() }
async function body(result) {
  const length = Number(result.headers.get('content-length'))
  assert.ok(!Number.isFinite(length) || length <= 8 * 1024 * 1024, 'Response exceeds verification budget')
  const text = await result.text()
  assert.ok(Buffer.byteLength(text) <= 8 * 1024 * 1024, 'Response exceeds verification budget')
  return text
}
async function check(label, action) {
  try { await action(); checks.push({ label, pass: true }); console.log(`PASS ${label}`) }
  catch (error) {
    checks.push({ label, pass: false })
    // Assertions use fixed diagnostics only: never print response bodies or request headers.
    console.error(`FAIL ${label}: ${error instanceof assert.AssertionError ? error.message : 'request or response processing failed'}`)
  }
}
async function jsonHealth(path, expected) {
  const result = await response(gateway, path)
  assert.equal(result.status, 200, 'Health request must return 200')
  assert.equal(type(result), 'application/json', 'Health response must be JSON')
  const data = JSON.parse(await body(result))
  for (const [field, value] of Object.entries(expected)) assert.equal(data[field], value, 'Health response is not ready')
}
function releaseUrl(value) {
  assert.equal(typeof value, 'string', 'Asset URL must be a string')
  const url = new URL(value, new URL(indexPath, gateway))
  assert.equal(url.origin, gateway, 'Release asset must use the public gateway')
  assert.ok(url.pathname.startsWith(releasePath) && !url.search && !url.hash, 'Release asset must remain in its immutable version')
  return url.pathname
}

await check('public platform HTML', async () => {
  const result = await response(gateway, '/micro/stores')
  assert.equal(result.status, 200, 'Platform navigation must return 200')
  assert.equal(type(result), 'text/html', 'Platform navigation must return HTML')
  assert.match(await body(result), /<div\s+[^>]*id=["']root["']/i, 'Platform root element is missing')
})
await check('public platform API health', () => jsonHealth('/api/health', { status: 'UP', database: 'UP' }))
await check('public stores readiness', () => jsonHealth('/stores-api/health/ready', { status: 'ok', service: 'stores-api' }))
await check('unauthenticated business API is denied', async () => {
  const result = await response(gateway, '/stores-api/stores', { headers: { Origin: 'null' } })
  assert.equal(result.status, 401, 'Unauthenticated stores request must return 401')
  assert.equal(type(result), 'application/json', 'Business error must be JSON, not a platform SPA fallback')
  assert.equal(result.headers.get('access-control-allow-origin'), 'null', 'Allowed iframe origin must remain readable')
  await result.body?.cancel()
})
await check('opaque iframe preflight reaches the business API', async () => {
  const result = await response(gateway, '/stores-api/stores', { method: 'OPTIONS', headers: {
    Origin: 'null', 'Access-Control-Request-Method': 'POST',
    'Access-Control-Request-Headers': 'authorization,content-type,x-request-id',
  } })
  assert.equal(result.status, 204, 'Allowed preflight must return 204')
  assert.equal(result.headers.get('access-control-allow-origin'), 'null', 'Preflight must allow only the requested configured origin')
  assert.match(result.headers.get('access-control-allow-methods') || '', /(?:^|,\s*)POST(?:,|$)/, 'POST missing from allowed methods')
  const allowed = (result.headers.get('access-control-allow-headers') || '').toLowerCase().split(',').map(value => value.trim())
  for (const required of ['authorization', 'content-type', 'x-request-id']) assert.ok(allowed.includes(required), 'Required request header missing')
})
await check('unapproved origin is rejected', async () => {
  const result = await response(gateway, '/stores-api/stores', { method: 'OPTIONS', headers: {
    Origin: 'https://not-authorized.invalid', 'Access-Control-Request-Method': 'GET',
    'Access-Control-Request-Headers': 'authorization',
  } })
  assert.equal(result.status, 403, 'Unapproved origin must return 403')
  assert.equal(result.headers.get('access-control-allow-origin'), null, 'Gateway must not reflect an unapproved origin')
  await result.body?.cancel()
})
await check('private business routes are hidden by the public gateway', async () => {
  for (const path of ['/stores-api/internal', '/stores-api/internal/metrics', '/stores-api/metrics', '/stores-api/metrics/nested']) {
    const result = await response(gateway, path)
    assert.equal(result.status, 404, 'Private business route must return 404')
    await result.body?.cancel()
  }
})
await check('versioned HTML and JS/CSS are served independently', async () => {
  const result = await response(gateway, indexPath, { headers: { Origin: 'null' } })
  assert.equal(result.status, 200, 'Release index must return 200')
  assert.equal(type(result), 'text/html', 'Release index must have an HTML MIME type')
  assert.equal(result.headers.get('access-control-allow-origin'), '*', 'Public static HTML must support iframe loading')
  assert.match(result.headers.get('cache-control') || '', /no-cache/, 'HTML must revalidate')
  const html = await body(result)
  assert.match(html, /<div\s+[^>]*id=["']app["']/i, 'Stores app root is missing')
  const assets = new Set(Array.from(html.matchAll(/<script\b[^>]*\bsrc=["']([^"']+)["']/gi), match => releaseUrl(match[1])))
  const manifestResult = await response(gateway, `${releasePath}asset-manifest.json`)
  assert.equal(manifestResult.status, 200, 'Release asset manifest must return 200')
  const manifest = JSON.parse(await body(manifestResult))
  for (const entry of Object.values(manifest)) {
    if (entry && typeof entry.file === 'string' && /\.(?:js|mjs|css)$/.test(entry.file)) assets.add(releaseUrl(entry.file))
    for (const css of entry?.css || []) assets.add(releaseUrl(css))
  }
  assert.ok([...assets].some(path => /\.(?:js|mjs)$/.test(path)), 'JavaScript assets are missing')
  assert.ok([...assets].some(path => path.endsWith('.css')), 'Business CSS asset is missing')
  assert.ok(assets.size <= 48, 'Too many resources for the smoke check')
  for (const path of assets) {
    const asset = await response(gateway, path, { headers: { Origin: 'null' } })
    assert.equal(asset.status, 200, 'Versioned asset must return 200')
    assert.ok(path.endsWith('.css') ? type(asset) === 'text/css' : ['application/javascript', 'text/javascript'].includes(type(asset)), 'Asset has the wrong MIME type')
    assert.equal(asset.headers.get('access-control-allow-origin'), '*', 'Opaque iframe assets need public CORS')
    assert.match(asset.headers.get('cache-control') || '', /immutable/, 'Versioned assets must be immutable')
    assert.ok((await body(asset)).length > 0, 'Versioned asset is empty')
  }
})
await check('missing versioned asset stays 404', async () => {
  const result = await response(gateway, `${releasePath}assets/__gateway_smoke_missing__.js`)
  assert.equal(result.status, 404, 'Missing release asset must return 404 without SPA fallback')
  assert.doesNotMatch(result.headers.get('cache-control') || '', /immutable/, 'Missing asset must not be cached immutably')
  await result.body?.cancel()
})
await check('platform Vite does not serve the stores release', async () => {
  const result = await response(platform, indexPath)
  assert.equal(result.status, 404, 'Direct platform must not own the stores static release')
  await result.body?.cancel()
})
await check('platform Vite does not proxy the stores API', async () => {
  const result = await response(platform, '/stores-api/health/ready')
  assert.notEqual(type(result), 'application/json', 'Direct platform unexpectedly serves a stores JSON API')
  assert.ok([200, 404].includes(result.status), 'Direct platform response is unexpected')
  await result.body?.cancel()
})

const failures = checks.filter(check => !check.pass).length
console.log(`Gateway verification: ${checks.length - failures}/${checks.length} passed`)
if (failures) process.exitCode = 1
