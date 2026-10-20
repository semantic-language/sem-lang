import assert from 'node:assert/strict'
import { readMetadata } from './metadata.mts'
import { spawnSync } from 'node:child_process'
import { readdir, readFile } from 'node:fs/promises'

const root = new URL('../crates/', import.meta.url)
const rawArguments: unknown = JSON.parse(process.env.SEM_LANG_CARGO_ARGS_JSON ?? '[]')
assert.ok(Array.isArray(rawArguments), 'SEM_LANG_CARGO_ARGS_JSON must be a JSON string array')
const cargoConfiguration = (rawArguments as unknown[]).map(value => {
  assert.equal(typeof value, 'string', 'Cargo argument must be a string')
  return value as string
})
const metadata = spawnSync('cargo', ['metadata', ...cargoConfiguration, '--offline', '--no-deps', '--format-version', '1'], {
  cwd: new URL('..', import.meta.url), encoding: 'utf8', maxBuffer: 8 * 1024 * 1024
})
assert.equal(metadata.status, 0, metadata.stderr || metadata.error?.message)
for (const pkg of readMetadata(metadata.stdout).packages) {
  for (const dependency of pkg.dependencies) {
    assert.doesNotMatch(dependency.name, /^(?:hatter|hat-specifications|official-hats|crowsi|ihat)(?:-|$)/u,
      `${pkg.name}: aliased or inherited service dependency ${dependency.name} is forbidden`)
  }
}
let checked = 0
for (const entry of await readdir(root, { withFileTypes: true })) {
  if (!entry.isDirectory()) continue
  const manifest = await readFile(new URL(`${entry.name}/Cargo.toml`, root), 'utf8')
  assert.doesNotMatch(manifest, /^\s*(?:hatter|hat-specifications|official-hats|crowsi|ihat)[\w-]*\s*=/mu,
    `${entry.name}: a foundational package must not depend on a product or authority`)
  checked++
}
console.log(`sem-lang service boundary verified: ${checked} crate manifests`)
