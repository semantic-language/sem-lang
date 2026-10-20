import assert from 'node:assert/strict'
import { readMetadata } from './metadata.mts'
import { readFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'

const result = spawnSync('cargo', ['metadata', '--locked', '--no-deps', '--format-version', '1'], { encoding: 'utf8', maxBuffer: 8 * 1024 * 1024 })
if (result.status !== 0) throw new Error(result.stderr)
const metadata = readMetadata(result.stdout)
const owned = new Set(metadata.workspace_members)
for (const pkg of metadata.packages.filter(pkg => owned.has(pkg.id))) {
  assert.deepEqual(pkg.publish, [], `${pkg.name} must remain source-only`)
  if (pkg.repository !== null || pkg.name === 'sem-lang-inference-client') {
    assert.equal(pkg.repository, 'https://github.com/semantic-language/sem-lang', `${pkg.name} must identify the product source`)
  }
  for (const dep of pkg.dependencies) {
    if (dep.name.startsWith('sem-lang')) {
      assert.equal(dep.source, null, `${pkg.name}: ${dep.name} must resolve from owned source`)
      assert.ok(metadata.packages.some(candidate => owned.has(candidate.id) && candidate.name === dep.name && candidate.manifest_path === `${dep.path}/Cargo.toml`), `${dep.name} must belong to this workspace`)
    } else if (dep.name.startsWith('zixcel-')) {
      const expected = dep.name === 'zixcel-json-structure'
        ? 'git+https://github.com/zixcel/document-processing?rev=a171286c1676a8bbaa7d5c32e7cd131a293fcc4d'
        : 'registry+https://github.com/rust-lang/crates.io-index'
      assert.equal(dep.source, expected, `${dep.name} must use its reviewed public source`)
    }
  }
}
const lock = readFileSync('Cargo.lock', 'utf8')
assert.ok(!/ecosystem-private|zixcel-private/.test(lock), 'private registry lock entries are forbidden')
console.log(`Product dependency boundary verified: ${owned.size} source-owned packages`)
