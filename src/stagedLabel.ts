import type { StagedMod } from "./types";

function distinctFileName(modName: string, candidate: string | null | undefined): string | null {
  const name = candidate?.trim() ?? "";
  if (!name || name.toLowerCase() === modName.trim().toLowerCase()) return null;
  return name;
}

/** Chooser title when it differs from the mod, otherwise the archive name. */
export function chosenFileLabel(
  modName: string,
  file: { name?: string | null; file_name?: string | null },
): string | null {
  return (
    distinctFileName(modName, file.name) ?? distinctFileName(modName, file.file_name)
  );
}

/** Second line under a staged mod: the file name plus version, or the source line. */
export function stagedModSourceLine(mod: StagedMod): string {
  const source =
    mod.source === "thunderstore"
      ? `Thunderstore · ${mod.ts_namespace ?? "?"}-${mod.ts_name ?? "?"}`
      : mod.source === "modio"
        ? `mod.io · game ${mod.modio_game_id ?? "?"} · mod ${mod.modio_mod_id ?? "?"} · file ${mod.modio_file_id ?? "?"}`
        : `#${mod.nexus_mod_id} · file ${mod.nexus_file_id}`;
  const file = distinctFileName(mod.name, mod.file_name);
  if (file) {
    return mod.version ? `${file} · v${mod.version}` : file;
  }
  return mod.version ? `${source} · v${mod.version}` : source;
}
