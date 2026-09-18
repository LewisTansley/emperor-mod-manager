#!/usr/bin/env node
/**
 * Builds the distro-agnostic Linux tarball consumed by packaging/arch/PKGBUILD.
 * Tauri's own bundle names contain spaces, which GitHub rewrites on upload
 * (`Emperor.Mod.Manager_0.4.0_amd64.deb`); the PKGBUILD needs a stable name.
 */
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const pkg = JSON.parse(fs.readFileSync(path.join(repoRoot, "package.json"), "utf8"));
const version = pkg.version;
const stem = `emperor-mod-manager-${version}-x86_64`;

const binary = path.join(repoRoot, "src-tauri/target/release/emperor-mod-manager");
if (!fs.existsSync(binary)) {
  console.error(
    `[package-linux-tarball] missing ${binary}\nRun \`npm run tauri build\` first.`,
  );
  process.exit(1);
}

const outDir = path.join(repoRoot, "dist-packages");
const stageRoot = path.join(repoRoot, ".build-tmp", "linux-tarball");
const stage = path.join(stageRoot, stem);
fs.rmSync(stageRoot, { recursive: true, force: true });
fs.mkdirSync(path.join(stage, "icons"), { recursive: true });
fs.mkdirSync(outDir, { recursive: true });

fs.copyFileSync(binary, path.join(stage, "emperor-mod-manager"));
fs.chmodSync(path.join(stage, "emperor-mod-manager"), 0o755);
fs.copyFileSync(
  path.join(repoRoot, "src-tauri/emperor-mod-manager.desktop"),
  path.join(stage, "emperor-mod-manager.desktop"),
);
fs.copyFileSync(path.join(repoRoot, "LICENSE"), path.join(stage, "LICENSE"));

for (const [src, dest] of [
  ["32x32.png", "32x32.png"],
  ["128x128.png", "128x128.png"],
  ["128x128@2x.png", "256x256.png"],
]) {
  fs.copyFileSync(
    path.join(repoRoot, "src-tauri/icons", src),
    path.join(stage, "icons", dest),
  );
}

const tarball = path.join(outDir, `${stem}.tar.gz`);
fs.rmSync(tarball, { force: true });
const tar = spawnSync(
  "tar",
  ["--owner=0", "--group=0", "-czf", tarball, "-C", stageRoot, stem],
  { stdio: "inherit" },
);
if (tar.status !== 0) {
  console.error("[package-linux-tarball] tar failed");
  process.exit(tar.status ?? 1);
}
fs.rmSync(stageRoot, { recursive: true, force: true });

const sha256 = createHash("sha256").update(fs.readFileSync(tarball)).digest("hex");
fs.writeFileSync(path.join(outDir, "SHA256SUMS"), `${sha256}  ${stem}.tar.gz\n`);

console.log(`[package-linux-tarball] ${tarball}`);
console.log(`[package-linux-tarball] sha256 ${sha256}`);
