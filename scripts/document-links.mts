import assert from 'node:assert/strict'
import { posix, resolve, relative, join } from 'node:path'
import { readFileSync, readdirSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
interface Selected { target: string; bytes: Buffer }
export function checkLinks(selected: Selected[]): number {
  const targets = new Set(selected.map(entry => entry.target))
  let checked = 0
  for (const entry of selected.filter(value => value.target.endsWith('.md'))) {
    const text = entry.bytes.toString('utf8').replace(/```[\s\S]*?```/g, '')
    const links = [...text.matchAll(/\[[^\]]*\]\(([^)\s]+)(?:\s+[^)]*)?\)/g)].map(match => match[1]!)
    const codeReferences = [...text.matchAll(/`(\.\.?\/[^`\s]+\.md)`/g)].map(match => match[1]!)
    for (const href of [...new Set([...links, ...codeReferences])]) {
      if (/^[a-z]+:/i.test(href) || href.startsWith('#')) continue
      const path = posix.normalize(posix.join(posix.dirname(entry.target), decodeURIComponent(href.split(/[?#]/)[0]!)))
      assert(!path.startsWith('../') && !posix.isAbsolute(path), `Link escapes snapshot: ${entry.target}`)
      assert(targets.has(path) || selected.some(value => value.target.startsWith(path.replace(/\/$/, '') + '/')),
        `Link missing from public snapshot: ${entry.target} -> ${href}`)
      checked++
    }
  }
  return checked
}

export function sourceFiles(root: string, directory = root, selected: Selected[] = []): Selected[] {
  const excluded = new Set(['.git', '.local', 'target', 'node_modules', '.nuxt', '.output', 'dist'])
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (excluded.has(entry.name)) continue
    assert(!entry.isSymbolicLink(), 'Document source symlink refused')
    const path = join(directory, entry.name)
    if (entry.isDirectory()) sourceFiles(root, path, selected)
    else {
      assert(entry.isFile() && selected.length < 2000, 'Source file budget exceeded')
      const target = relative(root, path).split('\\').join('/')
      const bytes = target.endsWith('.md') ? readFileSync(path) : Buffer.alloc(0)
      assert(bytes.length <= 131072, 'Document byte budget exceeded')
      selected.push({ target, bytes })
    }
  }
  return selected
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  console.log(`Source document links checked: ${checkLinks(sourceFiles(process.cwd()))}`)
}
