import { dirname, resolve } from 'node:path'
import { readdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
function run(command: string, args: string[]): void {
  const result = spawnSync(command, args, { cwd: root, env: process.env, stdio: 'inherit', shell: false })
  if (result.error) throw result.error
  if (result.status !== 0) process.exit(result.status ?? 1)
}
run('pnpm', ['exec', 'tsc', '-p', 'tsconfig.tools.json'])
run(process.execPath, ['--test', ...readdirSync(resolve(root, 'scripts')).filter(name => name.endsWith('.test.mts')).sort().map(name => `scripts/${name}`)])
run('cargo', ['fmt', '--all', '--', '--check'])
run('node', ['scripts/check-source-budgets.mts'])
run('node', ['scripts/check-rust-path-boundaries.mts', '.'])
run('node', ['scripts/check-service-boundary.mts'])
run('node', ['scripts/check-product-dependencies.mts'])
// Check the facade independently before workspace feature unification can hide leaks.
run('cargo', ['check', '--locked', '-p', 'sem-lang', '--no-default-features', '--features', 'dsl,memory-store', '--lib'])
run('cargo', ['check', '--locked', '-p', 'sem-lang', '--no-default-features', '--features', 'dsl,memory-store,json', '--lib'])
run('cargo', ['test', '--locked', '--workspace', '--all-targets'])
run('cargo', ['test', '--locked', '--workspace', '--all-targets', '--all-features'])
run('cargo', ['clippy', '--locked', '--workspace', '--all-targets', '--all-features', '--', '-D', 'warnings'])
run('cargo', ['test', '--locked', '--manifest-path', 'conformance/source/Cargo.toml'])
run('cargo', ['run', '--locked', '--manifest-path', 'fixtures/source-consumer/Cargo.toml'])
