#!/usr/bin/env node
/**
 * Removes Cargo/Tauri build artifacts and local scratch dirs.
 * Does not touch source or node_modules.
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const ARTIFACT_DIRS = [
  "src-tauri/target",
  "src-tauri/src-tauri",
  "target",
  "dist",
  "dist-packages",
  ".tmp",
  ".build-tmp",
  ".cargo-tmp",
  ".npm-cache",
  ".cache",
];

function dirSizeBytes(dir) {
  let total = 0;
  const stack = [dir];
  while (stack.length > 0) {
    const current = stack.pop();
    let entries;
    try {
      entries = fs.readdirSync(current, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      const full = path.join(current, entry.name);
      if (entry.isDirectory()) {
        stack.push(full);
      } else if (entry.isFile() || entry.isSymbolicLink()) {
        try {
          total += fs.lstatSync(full).size;
        } catch {
          // ignore races / unreadable files
        }
      }
    }
  }
  return total;
}

function formatBytes(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

let reclaimed = 0;
let removed = 0;
let skipped = 0;

console.log("[clean] Removing build artifacts…");

for (const rel of ARTIFACT_DIRS) {
  const abs = path.join(repoRoot, rel);
  if (!fs.existsSync(abs)) {
    console.log(`  skip  ${rel} (missing)`);
    skipped += 1;
    continue;
  }
  const size = dirSizeBytes(abs);
  fs.rmSync(abs, { recursive: true, force: true });
  console.log(`  remove ${rel} (${formatBytes(size)})`);
  reclaimed += size;
  removed += 1;
}

console.log(
  `[clean] Done. Removed ${removed} path(s), skipped ${skipped}. Reclaimed ~${formatBytes(reclaimed)}.`,
);
console.log("[clean] Next `tauri` / Cargo build will recompile from scratch.");
