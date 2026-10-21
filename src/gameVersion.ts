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

export function browseGameVersionOptions(
  meta: BrowseMeta | null | undefined,
  mode: "mods" | "collections",
): string[] {
  if (!meta) return [];
  if (mode === "collections") return meta.game_versions;
  const fromTags = meta.mod_tags.filter(looksLikeGameVersion);
  return fromTags.length > 0 ? fromTags : meta.game_versions;
}
