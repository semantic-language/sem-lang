import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { join } from 'node:path'

const root = fileURLToPath(new URL('../', import.meta.url))
const read = (path: string): string => readFileSync(join(root, path), 'utf8')

test('first-party package metadata and component terms agree on MPL', () => {
  const workspace = read('Cargo.toml')
  assert.match(workspace, /license = "MPL-2.0"/)
  const manifests = [
    ...readdirSync(join(root, 'crates')).map(name => `crates/${name}/Cargo.toml`),
    ...readdirSync(join(root, 'fixtures')).map(name => `fixtures/${name}/Cargo.toml`),
    'conformance/source/Cargo.toml',
  ]
  for (const manifest of manifests) {
    assert.match(read(manifest), /license(?: = "MPL-2.0"|\.workspace = true)/, manifest)
    assert.match(read(manifest), /publish = false/, manifest)
  }
  for (const component of ['crates/sem-lang', 'crates/sem-lang-inference-client']) {
    assert.equal(read(`${component}/LICENSE`), read('LICENSE'))
    assert.match(read(`${component}/NOTICE`), /Previous grants and third-party notices/)
  }
  assert.match(read('LICENSE'), /Mozilla Public License Version 2.0/)
  assert.match(read('LICENSE-APACHE'), /END OF TERMS AND CONDITIONS/)
  assert.match(read('LICENSE-MIT'), /Permission is hereby granted/)
})
