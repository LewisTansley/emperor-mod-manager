import { describe, expect, it } from "vitest";
import {
  catalogFilterMismatchMessage,
  clampCatalogSourceFilter,
  configuredCatalogLabels,
  sourceFilterAllowed,
} from "./catalogSources";

const nexusOnly = {
  nexus_domain: "skyrimspecialedition",
  thunderstore_community: null,
  modio_game_id: null,
};

const multiSource = {
  nexus_domain: "valheim",
  thunderstore_community: "valheim",
  modio_game_id: 123,
};

describe("clampCatalogSourceFilter", () => {
  it("keeps a filter that the active game supports", () => {
    expect(clampCatalogSourceFilter("modio", multiSource, "mods")).toBe(
      "modio",
    );
    expect(
      clampCatalogSourceFilter("thunderstore", multiSource, "mods"),
    ).toBe("thunderstore");
  });

  it("resets to all when the game lacks that source", () => {
    expect(clampCatalogSourceFilter("modio", nexusOnly, "mods")).toBe("all");
    expect(
      clampCatalogSourceFilter("thunderstore", nexusOnly, "mods"),
    ).toBe("all");
  });

  it("resets mod.io when browsing collections", () => {
    expect(clampCatalogSourceFilter("modio", multiSource, "collections")).toBe(
      "all",
    );
  });

  it("keeps all even when the game has no sources", () => {
    expect(clampCatalogSourceFilter("all", {}, "mods")).toBe("all");
  });
});

describe("sourceFilterAllowed", () => {
  it("treats empty strings and zero ids as missing", () => {
    const empty = {
      nexus_domain: "",
      thunderstore_community: "",
      modio_game_id: 0,
    };
    expect(sourceFilterAllowed("nexus", empty, "mods")).toBe(false);
    expect(sourceFilterAllowed("thunderstore", empty, "mods")).toBe(false);
    expect(sourceFilterAllowed("modio", empty, "mods")).toBe(false);
    expect(sourceFilterAllowed("all", empty, "mods")).toBe(true);
  });
});

describe("catalogFilterMismatchMessage", () => {
  it("matches the Nexus-only / mod.io filter case", () => {
    expect(catalogFilterMismatchMessage("modio", nexusOnly)).toBe(
      "No results for the mod.io filter — this game only has Nexus configured. Switch source filter to All or Nexus.",
    );
  });

  it("lists every configured source", () => {
    expect(configuredCatalogLabels(multiSource)).toEqual([
      "Nexus",
      "Thunderstore",
      "mod.io",
    ]);
    expect(catalogFilterMismatchMessage("nexus", multiSource)).toContain(
      "Nexus / Thunderstore / mod.io",
    );
  });

  it("falls back to the generic empty-sources message", () => {
    expect(catalogFilterMismatchMessage("all", {})).toContain(
      "No catalog sources configured",
    );
  });
});
