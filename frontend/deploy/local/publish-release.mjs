import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { existsSync, mkdtempSync, realpathSync, rmSync } from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { physicalDirectory, requireReleaseVersion, verifyReleaseDirectory } from '../verify-release.mjs'
import { installRelease } from './install-release.mjs'

const options = {}
for (let index = 2; index < process.argv.length; index += 2) {
  const name = process.argv[index], value = process.argv[index + 1]
  assert.ok(['--version', '--allowed-host-origin', '--api-base'].includes(name) && value && !options[name], 'Use each public release option once with an explicit value')
  options[name] = value
}
const version = requireReleaseVersion(options['--version'])
assert.ok(options['--allowed-host-origin'], 'An explicit local host origin is required')
const frontend = fileURLToPath(new URL('../../', import.meta.url))
const releases = physicalDirectory(fileURLToPath(new URL('../../../artifacts/micro-apps/stores/versions/', import.meta.url)))
const destination = path.join(releases, version)
assert.ok(!existsSync(destination), 'Immutable release already exists; choose a new version')
const temporaryParent = realpathSync(os.tmpdir())
const temporary = realpathSync(mkdtempSync(path.join(temporaryParent, 'stores-release-')))
try {
  const output = path.join(temporary, 'release')
  execFileSync(process.execPath, [fileURLToPath(new URL('../../scripts/build.mjs', import.meta.url)),
    '--version', version, '--allowed-host-origin', options['--allowed-host-origin'],
    '--api-base', options['--api-base'] || '/stores-api', '--out-dir', output],
  { cwd: frontend, stdio: 'inherit', windowsHide: true })
  verifyReleaseDirectory(output, version)
  assert.ok(!existsSync(destination), 'Immutable release appeared during the build; choose a new version')
  installRelease(output, releases, version)
  console.log(`Published unactivated local candidate: ${destination}`)
} finally {
  for (const [directory, parent] of [[temporary, temporaryParent]]) {
    if (!existsSync(directory)) continue
    assert.equal(path.dirname(realpathSync(directory)), parent, 'Cleanup must remain in the owned build or staging parent')
    rmSync(directory, { recursive: true, force: true })
  }
}
