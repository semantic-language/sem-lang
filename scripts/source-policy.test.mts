import test from 'node:test'
import assert from 'node:assert/strict'
import { fileURLToPath } from 'node:url'
import { sourceFiles } from './document-links.mts'
test('first-party source tooling has no handwritten JavaScript entrypoints', () => {
  const files = sourceFiles(fileURLToPath(new URL('../', import.meta.url)))
  assert.deepEqual(files.filter(file => /\.(?:[cm]?js|jsx)$/.test(file.target)).map(file => file.target), [])
})
