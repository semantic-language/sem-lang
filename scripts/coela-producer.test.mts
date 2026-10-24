import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { createHash } from 'node:crypto'
const base = new URL('../conformance/source/fixtures/coela-evidence/', import.meta.url)
const hash = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex')
test('independent native consumer selects exact public Coela source and original attribution', () => {
  const provenance = JSON.parse(readFileSync(new URL('provenance.json', base), 'utf8'))
  const source = readFileSync(new URL('evidence.sem', base))
  const descriptor = readFileSync(new URL('source.json', base))
  assert.equal(provenance.repository, 'https://github.com/coela-oss/engineering')
  assert.match(provenance.source, /^[a-f0-9]{40}$/)
  assert.match(provenance.release, /^https:\/\/github.com\/coela-oss\/engineering\/releases\/tag\/semantic\/v0\.1\.0$/)
  assert.match(provenance.archiveSha256, /^[a-f0-9]{64}$/)
  assert.equal(hash(source), provenance.files['integrations/semantic/evidence.sem'])
  assert.equal(hash(descriptor), provenance.files['integrations/semantic/source.json'])
  assert.equal(JSON.parse(descriptor.toString()).module.sha256, hash(source))
  assert.equal(provenance.attribution, 'Coela')
  assert.equal(provenance.license, 'CC-BY-4.0')
  assert.equal(provenance.descriptorLicense, 'MIT')
  assert.match(readFileSync(new URL('LICENSE-MIT.txt', base), 'utf8'), /Permission is hereby granted/)
  assert.match(readFileSync(new URL('LICENSE-CC-BY-4.0.txt', base), 'utf8'), /Creative Commons/)
})
