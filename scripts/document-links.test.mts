import test from 'node:test'
import assert from 'node:assert/strict'
import { checkLinks } from './document-links.mts'
const entry = (target: string, text = '') => ({ target, bytes: Buffer.from(text) })
test('snapshot-local links resolve only against the selected target set', () => {
  assert.equal(checkLinks([entry('conformance/source/README.md', '[Intro](../../docs/start.md) [crate](../../crates/core)'),
    entry('docs/start.md'), entry('crates/core/Cargo.toml')]), 2)
  assert.throws(() => checkLinks([entry('examples/editor/README.md', '[Private](../../docs/private.md)')]), /missing/)
  assert.throws(() => checkLinks([entry('conformance/source/README.md', '`../../docs/private.md`')]), /missing/)
})
test('fenced code, fragments and external sources are not mistaken for distributed files', () => {
  assert.equal(checkLinks([entry('README.md', '[web](https://example.test) [section](#heading)\n```md\n[example](missing.md)\n```')]), 0)
  assert.throws(() => checkLinks([entry('README.md', '[escape](../private.md)')]), /escapes/)
})
