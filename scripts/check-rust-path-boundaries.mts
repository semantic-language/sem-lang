import { readdirSync, readFileSync, lstatSync } from "node:fs";
import { join, relative, resolve } from "node:path";

const root = resolve(process.argv[2] ?? ".");
const sourceRoots = [
  "crates",
  "conformance",
  "benches",
  "fixtures",
  "fuzz",
  "test-support",
];
const checks = [
  {
    label: "Unix host path",
    pattern: /["']\/(?:home|Users|tmp|var|opt|usr|mnt)\//g,
  },
  {
    label: "Windows host path",
    pattern: /["'][A-Za-z]:\\/g,
  },
  {
    label: "Linux process filesystem path",
    pattern: /["']\/proc\//g,
  },
  {
    label: "implicit current-directory fallback",
    pattern: /Path::new\(\s*"\."\s*\)/g,
  },
];

function rustFiles(path: string): string[] {
  const kind = lstatSync(path);
  if (kind.isSymbolicLink()) throw Error("Source root symlink refused");
  if (!kind.isDirectory()) return [];
  return readdirSync(path, { withFileTypes: true }).flatMap((entry) => {
    const child = join(path, entry.name);
    if (entry.isSymbolicLink()) throw Error("Source entry symlink refused");
    if (entry.isDirectory()) return rustFiles(child);
    return entry.isFile() && entry.name.endsWith(".rs") ? [child] : [];
  });
}

const failures: string[] = [];
let checked = 0;
for (const sourceRoot of sourceRoots) {
  const path = join(root, sourceRoot);
  let files: string[] = [];
  try {
    files = rustFiles(path);
  } catch (error) {
    if (!error || typeof error !== "object" || !("code" in error) || error.code !== "ENOENT") throw error;
  }
  for (const file of files) {
    checked += 1;
    const lines = readFileSync(file, "utf8").split("\n");
    lines.forEach((line, index) => {
      for (const check of checks) {
        check.pattern.lastIndex = 0;
        if (check.pattern.test(line)) {
          failures.push(
            `${relative(root, file)}:${index + 1}: ${check.label}`,
          );
        }
      }
    });
  }
}

if (failures.length > 0) {
  process.stderr.write(`${failures.join("\n")}\n`);
  process.exitCode = 1;
} else {
  process.stdout.write(`Rust path boundary check passed (${checked} files).\n`);
}
