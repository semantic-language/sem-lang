interface Dependency { name: string; source: string | null; path?: string }
interface Package { id: string; name: string; repository: string | null; publish: string[] | null
  manifest_path: string; dependencies: Dependency[] }
export interface CargoMetadata { workspace_members: string[]; packages: Package[] }
function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw Error('Invalid Cargo metadata object')
  return value as Record<string, unknown>
}
function string(value: unknown): string {
  if (typeof value !== 'string') throw Error('Invalid Cargo metadata string')
  return value
}
function strings(value: unknown): string[] {
  if (!Array.isArray(value)) throw Error('Invalid Cargo metadata array')
  return value.map((entry: unknown) => string(entry))
}
function nullable(value: unknown): string | null { return value === null ? null : string(value) }
function dependency(value: unknown): Dependency {
  const row = record(value)
  return { name: string(row.name), source: nullable(row.source),
    ...(row.path === undefined ? {} : { path: string(row.path) }) }
}
export function readMetadata(text: string): CargoMetadata {
  let value: unknown
  try { value = JSON.parse(text) as unknown } catch { throw Error('Invalid Cargo metadata JSON') }
  const root = record(value)
  if (!Array.isArray(root.packages)) throw Error('Invalid Cargo metadata packages')
  return { workspace_members: strings(root.workspace_members), packages: root.packages.map((value: unknown) => {
    const row = record(value)
    if (!Array.isArray(row.dependencies)) throw Error('Invalid Cargo metadata dependencies')
    return { id: string(row.id), name: string(row.name), repository: nullable(row.repository),
      publish: row.publish === null ? null : strings(row.publish), manifest_path: string(row.manifest_path),
      dependencies: row.dependencies.map((entry: unknown) => dependency(entry)) }
  }) }
}
