export type CatalogSourceFilter = "all" | "nexus" | "thunderstore" | "modio";

export type CatalogGameSources = {
  nexus_domain?: string | null;
  thunderstore_community?: string | null;
  modio_game_id?: number | null;
};

export function hasNexusSource(
  game: CatalogGameSources | null | undefined,
): boolean {
  return Boolean(game?.nexus_domain);
}

export function hasThunderstoreSource(
  game: CatalogGameSources | null | undefined,
): boolean {
  return Boolean(game?.thunderstore_community);
}

export function hasModioSource(
  game: CatalogGameSources | null | undefined,
): boolean {
  return game?.modio_game_id != null && game.modio_game_id > 0;
}

export function configuredCatalogLabels(
  game: CatalogGameSources | null | undefined,
): string[] {
  const labels: string[] = [];
  if (hasNexusSource(game)) {
    labels.push("Nexus");
  }
  if (hasThunderstoreSource(game)) {
    labels.push("Thunderstore");
  }
  if (hasModioSource(game)) {
    labels.push("mod.io");
  }
  return labels;
}

export function sourceFilterAllowed(
  filter: CatalogSourceFilter,
  game: CatalogGameSources | null | undefined,
  mode: "mods" | "collections",
): boolean {
  if (filter === "all") {
    return true;
  }
  if (filter === "nexus") {
    return hasNexusSource(game);
  }
  if (filter === "thunderstore") {
    return hasThunderstoreSource(game);
  }
  if (filter === "modio") {
    return mode === "mods" && hasModioSource(game);
  }
  return false;
}

export function clampCatalogSourceFilter(
  filter: CatalogSourceFilter,
  game: CatalogGameSources | null | undefined,
  mode: "mods" | "collections",
): CatalogSourceFilter {
  return sourceFilterAllowed(filter, game, mode) ? filter : "all";
}

function filterLabel(filter: CatalogSourceFilter): string {
  switch (filter) {
    case "nexus":
      return "Nexus";
    case "thunderstore":
      return "Thunderstore";
    case "modio":
      return "mod.io";
    default:
      return filter;
  }
}

export function catalogFilterMismatchMessage(
  filter: CatalogSourceFilter,
  game: CatalogGameSources | null | undefined,
): string {
  const configured = configuredCatalogLabels(game);
  if (configured.length === 0) {
    return "No catalog sources configured. Set a Nexus domain, Thunderstore community, and/or mod.io game ID.";
  }
  const label = filterLabel(filter);
  const joined = configured.join(" / ");
  return `No results for the ${label} filter — this game only has ${joined} configured. Switch source filter to All or ${joined}.`;
}
