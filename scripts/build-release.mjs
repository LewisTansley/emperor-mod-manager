#!/usr/bin/env node
/**
 * Local `build:release` builds the host OS only.
 * Dual Linux + Windows binaries are produced in parallel by CI
 * (.github/workflows/build.yml) on the same workflow run.
 */
import { spawnSync } from "node:child_process";
import process from "node:process";

const platform = process.platform;
const dualHint =
  "Both Linux and Windows packages are built in parallel by GitHub Actions " +
  "(.github/workflows/build.yml). This command builds the current host only.";

console.log(`[build:release] host=${platform}`);
console.log(`[build:release] ${dualHint}`);

const result = spawnSync("npx", ["tauri", "build"], {
  stdio: "inherit",
  shell: true,
  env: process.env,
});

process.exit(result.status ?? 1);
