import assert from 'node:assert/strict'
import { constants, copyFileSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { physicalDirectory, requireReleaseVersion, verifyReleaseDirectory } from '../verify-release.mjs'

export function installRelease(source, releaseRoot, version) {
  requireReleaseVersion(version)
  const input = physicalDirectory(source), releases = physicalDirectory(releaseRoot)
  verifyReleaseDirectory(input, version)
  assert.ok(!existsSync(path.join(input, 'release-complete.json')), 'Install source must be an unpublished build without a completion marker')
  const destination = path.join(releases, version)
  // mkdir is the no-replace reservation, including when another publisher races
  // or an incomplete/empty version already exists. Never rename over a version.
  mkdirSync(destination)
  function copy(directory, target) {
    physicalDirectory(directory)
    physicalDirectory(target)
    for (const item of readdirSync(directory, { withFileTypes: true })) {
      const original = path.join(directory, item.name), output = path.join(target, item.name)
      assert.ok(!lstatSync(original).isSymbolicLink(), 'Release files must not be symbolic links')
      if (item.isDirectory()) { mkdirSync(output); copy(original, output) }
      else {
        assert.ok(item.isFile(), 'Release input must be a regular file')
        copyFileSync(original, output, constants.COPYFILE_EXCL)
      }
    }
  }
  // A failed reserved version is deliberately retained without a completion
  // marker for inspection. Retrying it must fail; select a new version.
  copy(input, destination)
  verifyReleaseDirectory(destination, version)
  const info = JSON.parse(readFileSync(path.join(destination, 'build-info.json'), 'utf8'))
  writeFileSync(path.join(destination, 'release-complete.json'), JSON.stringify({
    complete: true, version, sourceSha256: info.sourceSha256,
  }, null, 2) + '\n', { flag: 'wx' })
  verifyReleaseDirectory(destination, version, { published: true })
  return destination
}
