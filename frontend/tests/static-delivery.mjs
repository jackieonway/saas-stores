import assert from 'node:assert/strict'
import test from 'node:test'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, realpathSync, rmSync } from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'
import { preview } from 'vite'
import { createStoresConfig, readPublicConfig } from '../vite.config.mjs'

test('version-prefixed static HTTP serves anonymous Origin:null ESM/chunks/CSS and preserves missing-resource 404', { timeout: 90000 }, async t => {
  const temporary = realpathSync(os.tmpdir()), root = realpathSync(mkdtempSync(path.join(temporary, 'platform-stores-static-')))
  t.after(() => {
    assert.equal(path.dirname(root), temporary)
    assert.ok(path.basename(root).startsWith('platform-stores-static-'))
    assert.equal(realpathSync(root), root)
    rmSync(root, { recursive: true, force: true })
  })
  const outDir = path.join(root, 'release'), builder = fileURLToPath(new URL('../scripts/build.mjs', import.meta.url))
  const built = spawnSync(process.execPath, [builder, '--version', '1.0.0-static-test', '--out-dir', outDir, '--allowed-host-origin', 'https://platform.example.com'], { encoding: 'utf8', windowsHide: true, timeout: 90000 })
  assert.equal(built.status, 0, built.stdout + built.stderr)
  const prefix = '/sub-apps/stores/versions/1.0.0-static-test/'
  const config = createStoresConfig(readPublicConfig({ version: '1.0.0-static-test', origins: ['https://platform.example.com'] }))
  const server = await preview({ ...config, configFile: false, base: prefix, build: { ...config.build, outDir }, preview: { ...config.preview, port: 0 } })
  t.after(async () => {
    server.httpServer.closeAllConnections()
    await new Promise((resolve, reject) => server.httpServer.close(error => error ? reject(error) : resolve()))
  })
  const base = new URL(prefix, `http://127.0.0.1:${server.httpServer.address().port}`)
  const htmlResponse = await fetch(new URL('index.html', base))
  assert.equal(htmlResponse.status, 200)
  const html = await htmlResponse.text(), urls = [...html.matchAll(/(?:src|href)="(\.\/[^"#]+)"/g)].map(match => match[1])
  assert.ok(urls.length >= 2)
  const manifest = JSON.parse(readFileSync(path.join(outDir, 'asset-manifest.json'), 'utf8'))
  for (const record of Object.values(manifest)) urls.push(record.file, ...(record.css || []))
  for (const resource of new Set(urls)) {
    const url = new URL(resource, base)
    assert.ok(url.pathname.startsWith(prefix))
    const response = await fetch(url, { headers: { Origin: 'null' }, credentials: 'omit' })
    assert.equal(response.status, 200, resource)
    assert.equal(response.headers.get('access-control-allow-origin'), '*')
    assert.equal(response.headers.get('access-control-allow-credentials'), null)
    assert.equal(response.headers.get('set-cookie'), null)
    assert.match(response.headers.get('content-type'), resource.endsWith('.css') ? /text\/css/ : /javascript/)
    assert.equal(await response.text(), readFileSync(path.join(outDir, resource), 'utf8'))
  }
  for (const missing of ['assets/missing.js', 'assets/missing.css', 'missing.html', 'stores/arbitrary']) {
    const response = await fetch(new URL(missing, base), { headers: { Accept: 'text/html', Origin: 'null' } })
    assert.equal(response.status, 404, missing)
    assert.ok(!(await response.text()).includes('正在连接当前组织的门店空间'))
  }
})
