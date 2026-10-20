import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

// These are shrinking ceilings, not target sizes. A responsibility split must
// lower its ceiling in the same change; adding more behavior to an existing
// orchestrator is rejected. Derived dist/ files are intentionally excluded.
const budgets = new Map([
  ['crates/sem-lang-tooling/src/lib.rs', { lines: 2810, bytes: 125_000 }],
  ['crates/sem-lang-tooling/src/workspace_graph.rs', { lines: 2150, bytes: 87_000 }],
  ['crates/sem-lang-tooling/src/workspace_change.rs', { lines: 1610, bytes: 59_000 }],
])

for (const [relative, budget] of budgets) {
  const source = await readFile(new URL(`../${relative}`, import.meta.url), 'utf8')
  const lines = source.length === 0 ? 0 : source.split('\n').length
  const bytes = Buffer.byteLength(source)
  assert.ok(lines <= budget.lines,
    `${relative} has ${lines} lines; split a responsibility instead of exceeding ${budget.lines}`)
  assert.ok(bytes <= budget.bytes,
    `${relative} has ${bytes} bytes; split a responsibility instead of exceeding ${budget.bytes}`)
}

console.log(`sem-lang shrinking source budgets verified: ${budgets.size} orchestrators`)
