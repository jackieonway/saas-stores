import { readdirSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('../', import.meta.url))
const files = readdirSync(new URL('../tests/', import.meta.url)).filter(name => /(?:\.test\.(?:mjs|ts)|^(?:build|static-delivery)\.mjs)$/.test(name)).sort().map(name => `tests/${name}`)
if (!files.length) throw new Error('门店测试文件缺失')
const result = spawnSync(process.execPath, ['--test', '--test-concurrency=1', ...process.argv.slice(2), ...files], { cwd: root, stdio: 'inherit', windowsHide: true })
if (result.error) throw result.error
process.exitCode = result.status ?? 1
