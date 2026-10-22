import type { BrowseMeta } from "./types";

/** Same rule as the Nexus tag check: a short token with a digit and a dot. */
export function looksLikeGameVersion(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed || trimmed.length > 24 || !trimmed.includes(".")) return false;
  let digit = false;
  for (const c of trimmed) {
    const ok =
      (c >= "0" && c <= "9") ||
      (c >= "a" && c <= "z") ||
      (c >= "A" && c <= "Z") ||
      c === "." ||
      c === "-" ||
      c === "_" ||
      c === " ";
    if (!ok) return false;
    if (c >= "0" && c <= "9") digit = true;
  }
  return digit;
}

function numericParts(value: string): number[] | null {
  const trimmed = value.trim().replace(/^[vV]/, "");
  if (!trimmed.includes(".")) return null;
  const parts: number[] = [];
  for (const part of trimmed.split(".")) {
    if (!part || !/^\d+$/.test(part)) return null;
    parts.push(Number(part));
  }
  return parts.length >= 2 ? parts : null;
}

function aliasKey(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]/g, "");
}

/** A tag that names a compatibility line for a game with a known table. */
function knownLine(domain: string, value: string): string | null {
  const alias = aliasKey(value);
  if (domain === "skyrimspecialedition") {
    if (alias === "se") return "1.5";
    if (alias === "ae" || alias === "anniversary" || alias === "anniversaryedition") {
      return "1.6";
    }
    const parts = numericParts(value);
    if (!parts || parts[0] !== 1) return null;
    if (parts[1] === 5) return "1.5";
    if (parts[1] === 6) return "1.6";
    return null;
  }
  if (domain === "fallout4") {
    if (alias === "oldgen") return "old-gen";
    if (alias === "nextgen" || alias === "ng") return "next-gen";
    const parts = numericParts(value);
    if (!parts || parts.length < 3 || parts[0] !== 1 || parts[1] !== 10) return null;
    if (parts[2] <= 163) return "old-gen";
    if (parts[2] >= 980) return "next-gen";
    return null;
  }
  return null;
}

function hasKnownLines(domain: string | null | undefined): domain is string {
  const key = domain?.trim().toLowerCase() ?? "";
  return key === "skyrimspecialedition" || key === "fallout4";
}

export function browseGameVersionOptions(
  meta: BrowseMeta | null | undefined,
  mode: "mods" | "collections",
  domain?: string | null,
): string[] {
  if (!meta) return [];
  const domainKey = domain?.trim().toLowerCase() ?? "";
  const known = hasKnownLines(domainKey);
  const keep = (value: string) =>
    known ? knownLine(domainKey, value) != null : looksLikeGameVersion(value);
  if (mode === "collections") {
    return known ? meta.game_versions.filter(keep) : meta.game_versions;
  }
  const fromTags = meta.mod_tags.filter(keep);
  return fromTags.length > 0 ? fromTags : meta.game_versions.filter(keep);
}
