import type { CatalogHit, CatalogSource, ManagedGame } from "./types";

/** Payload emitted by the Rust `emperor-open-mod` event / take command. */
export type EmperorOpenMod =
  | { source: "nexus"; domain: string; mod_id: number }
  | {
      source: "thunderstore";
      community: string;
      namespace: string;
      package_name: string;
    }
  | { source: "modio"; game_id: number; mod_id: number };

function blankHit(source: CatalogSource, id: string): CatalogHit {
  return {
    source,
    id,
    name: id,
    summary: null,
    picture_url: null,
    author: null,
    downloads: null,
    endorsements: null,
    category: null,
    tags: [],
    mod_id: null,
    domain_name: null,
    community: null,
    namespace: null,
    package_name: null,
    full_name: null,
    package_url: null,
    rating_score: null,
    latest_version: null,
    modio_game_id: null,
    modio_mod_id: null,
    profile_url: null,
  };
}

/** Build a shareable `emperor://mod/...` link from a catalog hit. */
export function modDeepLinkFromHit(hit: CatalogHit): string | null {
  if (hit.source === "nexus") {
    const domain = hit.domain_name?.trim();
    const modId = hit.mod_id;
    if (!domain || modId == null || modId <= 0) return null;
    return `emperor://mod/nexus/${encodeURIComponent(domain)}/${modId}`;
  }
  if (hit.source === "thunderstore") {
    const community = hit.community?.trim();
    const namespace = hit.namespace?.trim();
    const packageName = hit.package_name?.trim();
    if (!community || !namespace || !packageName) return null;
    return `emperor://mod/thunderstore/${encodeURIComponent(community)}/${encodeURIComponent(namespace)}/${encodeURIComponent(packageName)}`;
  }
  if (hit.source === "modio") {
    const gameId = hit.modio_game_id;
    const modId = hit.modio_mod_id;
    if (gameId == null || gameId <= 0 || modId == null || modId <= 0) return null;
    return `emperor://mod/modio/${gameId}/${modId}`;
  }
  return null;
}

/** Minimal CatalogHit from a parsed emperor deep-link payload. */
export function catalogHitFromModDeepLink(payload: EmperorOpenMod): CatalogHit {
  if (payload.source === "nexus") {
    return {
      ...blankHit("nexus", `nexus:${payload.domain}:${payload.mod_id}`),
      name: `Mod ${payload.mod_id}`,
      mod_id: payload.mod_id,
      domain_name: payload.domain,
    };
  }
  if (payload.source === "thunderstore") {
    return {
      ...blankHit(
        "thunderstore",
        `thunderstore:${payload.community}:${payload.namespace}:${payload.package_name}`,
      ),
      name: `${payload.namespace}-${payload.package_name}`,
      community: payload.community,
      namespace: payload.namespace,
      package_name: payload.package_name,
      full_name: `${payload.namespace}-${payload.package_name}`,
    };
  }
  return {
    ...blankHit("modio", `modio:${payload.game_id}:${payload.mod_id}`),
    name: `Mod ${payload.mod_id}`,
    modio_game_id: payload.game_id,
    modio_mod_id: payload.mod_id,
  };
}

/** Find a managed game that matches the deep-link catalog identity. */
export function managedGameForModDeepLink(
  managed: ManagedGame[],
  payload: EmperorOpenMod,
): ManagedGame | null {
  if (payload.source === "nexus") {
    const domain = payload.domain.toLowerCase();
    return (
      managed.find((g) => g.nexus_domain?.toLowerCase() === domain) ?? null
    );
  }
  if (payload.source === "thunderstore") {
    const community = payload.community.toLowerCase();
    return (
      managed.find(
        (g) => g.thunderstore_community?.toLowerCase() === community,
      ) ?? null
    );
  }
  return (
    managed.find((g) => g.modio_game_id != null && g.modio_game_id === payload.game_id) ??
    null
  );
}

/** Parse an `emperor://mod/...` URL (for tests / paste support). */
export function parseModDeepLink(raw: string): EmperorOpenMod | null {
  const trimmed = raw.trim();
  if (!trimmed.toLowerCase().startsWith("emperor://")) return null;
  let url: URL;
  try {
    url = new URL(trimmed);
  } catch {
    return null;
  }
  if (url.protocol !== "emperor:") return null;

  const host = url.hostname;
  let parts = url.pathname.split("/").filter(Boolean);
  if (host.toLowerCase() === "mod") {
    // emperor://mod/nexus/... → hostname=mod, pathname=/nexus/...
  } else if (!host) {
    if (parts[0]?.toLowerCase() === "mod") {
      parts = parts.slice(1);
    } else {
      return null;
    }
  } else {
    return null;
  }

  const kind = parts[0]?.toLowerCase();
  if (kind === "nexus") {
    if (parts.length !== 3) return null;
    const domain = decodeURIComponent(parts[1] ?? "");
    const modId = Number(parts[2]);
    if (!domain || !Number.isInteger(modId) || modId <= 0) return null;
    return { source: "nexus", domain, mod_id: modId };
  }
  if (kind === "thunderstore") {
    if (parts.length !== 4) return null;
    const community = decodeURIComponent(parts[1] ?? "");
    const namespace = decodeURIComponent(parts[2] ?? "");
    const packageName = decodeURIComponent(parts[3] ?? "");
    if (!community || !namespace || !packageName) return null;
    return {
      source: "thunderstore",
      community,
      namespace,
      package_name: packageName,
    };
  }
  if (kind === "modio") {
    if (parts.length !== 3) return null;
    const gameId = Number(parts[1]);
    const modId = Number(parts[2]);
    if (
      !Number.isInteger(gameId) ||
      gameId <= 0 ||
      !Number.isInteger(modId) ||
      modId <= 0
    ) {
      return null;
    }
    return { source: "modio", game_id: gameId, mod_id: modId };
  }
  return null;
}
