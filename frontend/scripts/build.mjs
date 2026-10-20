import { existsSync, lstatSync, mkdirSync, readFileSync, realpathSync, rmSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import path from 'node:path'
import os from 'node:os'
import { fileURLToPath } from 'node:url'
import { build } from 'vite'
import { createStoresConfig, readPublicConfig, OUTPUT_OWNER } from '../vite.config.mjs'

const appRoot = fileURLToPath(new URL('../', import.meta.url))
const args = process.argv.slice(2), options = { origins: [] }
for (let index = 0; index < args.length; index += 2) {
  const name = args[index], value = args[index + 1]
  if (!value || !['--out-dir', '--version', '--allowed-host-origin', '--api-base'].includes(name)) throw new Error('未知构建参数或缺少值')
  if (name === '--allowed-host-origin') options.origins.push(value)
  else options[name.slice(2)] = value
}
const publicConfig = readPublicConfig(options)

// Inspect every existing path component before any recursive deletion. In
// particular, a junction under temp must not redirect cleanup outside temp.
function canonicalDestination(input) {
  let current = path.resolve(input), suffix = []
  while (!existsSync(current)) {
    suffix.unshift(path.basename(current))
    const parent = path.dirname(current)
    if (parent === current) throw new Error('输出目录无法解析')
    current = parent
  }
  for (let ancestor = current; ; ancestor = path.dirname(ancestor)) {
    if (lstatSync(ancestor).isSymbolicLink()) throw new Error('输出目录不得使用符号链接')
    if (path.dirname(ancestor) === ancestor) break
  }
  return path.join(realpathSync(current), ...suffix)
}
const destination = canonicalDestination(options['out-dir'] || path.join(appRoot, 'dist'))
const defaultDestination = path.join(realpathSync(appRoot), 'dist')
const temporary = realpathSync(os.tmpdir()), relative = path.relative(temporary, destination)
if (destination !== defaultDestination && (!relative || path.isAbsolute(relative) || relative === '..' || relative.startsWith('..' + path.sep))) {
  throw new Error('自定义输出须位于系统临时目录内；发布请打包生成的 dist')
}
if (existsSync(destination)) {
  let owned = false
  try {
    const info = JSON.parse(readFileSync(path.join(destination, 'build-info.json'), 'utf8'))
    owned = info.application === 'stores' && info.outputOwner === OUTPUT_OWNER && lstatSync(destination).isDirectory()
  } catch {}
  if (!owned) throw new Error('输出目录不属于本应用构建，拒绝覆盖')
}
// Fail typing before touching a previous valid release.
execFileSync(process.execPath, [path.join(appRoot, 'node_modules/typescript/bin/tsc'), '--noEmit', '-p', path.join(appRoot, 'tsconfig.json')], { stdio: 'inherit', windowsHide: true })
if (existsSync(destination)) rmSync(destination, { recursive: true, force: true })
mkdirSync(destination, { recursive: true })
const config = createStoresConfig(publicConfig)
await build({ ...config, configFile: false, build: { ...config.build, outDir: destination } })
console.log(`门店应用 ${publicConfig.version} 已构建：${destination}`)
