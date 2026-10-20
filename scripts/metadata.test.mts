import test from 'node:test'
import assert from 'node:assert/strict'
import { readMetadata } from './metadata.mts'
const fixture = () => ({ workspace_members: ['source'], packages: [{ id: 'source', name: 'semantic-source',
  repository: null, publish: [], manifest_path: '/synthetic/Cargo.toml',
  dependencies: [{ name: 'public-library', source: 'registry+public' }, { name: 'local-library', source: null, path: '/synthetic/local' }] }] })
test('Cargo metadata validates the fields actually used by source-boundary tools', () => {
  assert.deepEqual(readMetadata(JSON.stringify(fixture())), fixture())
  assert.deepEqual(readMetadata(JSON.stringify({ ...fixture(), future_field: true })), fixture())
})
test('malformed or partial Cargo metadata is rejected without echoing input values', () => {
  for (const value of [null, [], {}, { packages: [], workspace_members: [1] },
    { ...fixture(), packages: [{ ...fixture().packages[0], dependencies: [null] }] },
    { ...fixture(), packages: [{ ...fixture().packages[0], publish: 'public' }] }]) {
    assert.throws(() => readMetadata(JSON.stringify(value)), /Invalid Cargo metadata/)
  }
  assert.throws(() => readMetadata('private-sentinel-invalid-json'), error => error instanceof Error && !error.message.includes('private-sentinel'))
})
