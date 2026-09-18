#!/usr/bin/env node
/**
 * Stamps packaging/arch/PKGBUILD with a release version and the real tarball
 * checksum. CI runs this on the built tarball and attaches the result to the
 * GitHub release, so `makepkg` verifies the download instead of using SKIP.
 *
 * Usage: node scripts/arch-pkgbuild.mjs <tarball-or-dir> [-o <out-path>]
 */
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const outIndex = args.findIndex((a) => a === "-o" || a === "--out");
const outPath = outIndex === -1 ? null : args[outIndex + 1];
const input = args.filter((_, i) => i !== outIndex && i !== outIndex + 1)[0];

if (!input) {
  console.error("usage: node scripts/arch-pkgbuild.mjs <tarball-or-dir> [-o <out-path>]");
  process.exit(1);
}

const TARBALL = /^emperor-mod-manager-(.+)-x86_64\.tar\.gz$/;

function resolveTarball(target) {
  if (fs.statSync(target).isDirectory()) {
    const entries = fs
      .readdirSync(target, { recursive: true, withFileTypes: true })
      .filter((e) => e.isFile() && TARBALL.test(e.name));
    if (entries.length !== 1) {
      console.error(
        `[arch-pkgbuild] expected exactly one release tarball under ${target}, found ${entries.length}`,
      );
      process.exit(1);
    }
    return path.join(entries[0].parentPath ?? entries[0].path, entries[0].name);
  }
  return target;
}

const tarball = resolveTarball(input);
const version = TARBALL.exec(path.basename(tarball))?.[1];
if (!version) {
  console.error(`[arch-pkgbuild] cannot read a version out of ${tarball}`);
  process.exit(1);
}

const sha256 = createHash("sha256").update(fs.readFileSync(tarball)).digest("hex");
const template = fs.readFileSync(path.join(repoRoot, "packaging/arch/PKGBUILD"), "utf8");
const stamped = template
  .replace(/^pkgver=.*$/m, `pkgver=${version}`)
  .replace(
    /^# Releases publish a PKGBUILD asset.*\n# tracks main.*\n/m,
    "",
  )
  .replace(/^sha256sums=\('SKIP'\)$/m, `sha256sums=('${sha256}')`);

if (stamped === template) {
  console.error("[arch-pkgbuild] PKGBUILD substitution produced no changes");
  process.exit(1);
}

if (outPath) {
  fs.mkdirSync(path.dirname(path.resolve(outPath)), { recursive: true });
  fs.writeFileSync(outPath, stamped);
  console.log(`[arch-pkgbuild] wrote ${outPath} (pkgver=${version})`);
} else {
  process.stdout.write(stamped);
}
