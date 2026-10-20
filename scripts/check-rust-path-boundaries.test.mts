import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, symlinkSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
const script = new URL('./check-rust-path-boundaries.mts', import.meta.url)
const run = (root: string) => spawnSync(process.execPath, [script.pathname, root], { encoding: 'utf8', timeout: 5000, maxBuffer: 65536 })
test('path refusal reports location and category without printing source values', () => {
  const root = mkdtempSync(join(tmpdir(), 'semantic-path-check-')); mkdirSync(join(root, 'crates'))
  try {
    writeFileSync(join(root, 'crates/lib.rs'), 'const VALUE: &str = "/home/synthetic/private-sentinel";')
    const refused = run(root); assert.equal(refused.status, 1)
    assert.match(refused.stderr, /crates\/lib.rs:1: Unix host path/); assert(!refused.stderr.includes('private-sentinel'))
    writeFileSync(join(root, 'crates/lib.rs'), 'pub fn value() {}')
    assert.equal(run(root).status, 0)
  } finally { rmSync(root, { recursive: true, force: true }) }
})
test('source symlinks cannot bypass the path check by silently disappearing from its input', () => {
  const root = mkdtempSync(join(tmpdir(), 'semantic-symlink-check-')); mkdirSync(join(root, 'crates'))
  try {
    writeFileSync(join(root, 'valid.rs'), 'pub fn value() {}')
    symlinkSync(join(root, 'valid.rs'), join(root, 'crates/hidden.rs'))
    const refused = run(root); assert.notEqual(refused.status, 0); assert.match(refused.stderr, /symlink refused/)
  } finally { rmSync(root, { recursive: true, force: true }) }
})
