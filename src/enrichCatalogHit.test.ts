import { describe, expect, it } from "vitest";
import {
  enrichCatalogHitFromModio,
  enrichCatalogHitFromNexus,
  enrichCatalogHitFromThunderstore,
} from "./App";
import type { CatalogHit, ModDetail, ModioModDetail, TsPackageDetail } from "./types";

function hit(partial: Partial<CatalogHit> & Pick<CatalogHit, "source" | "id" | "name">): CatalogHit {
  return {
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
    ...partial,
  };
}

describe("enrichCatalogHitFromNexus", () => {
  it("replaces placeholder name and fills metadata", () => {
    const sparse = hit({
      source: "nexus",
      id: "nexus:warhammer40kdarktide:8",
      name: "Mod 8",
      mod_id: 8,
      domain_name: "warhammer40kdarktide",
    });
    const detail = {
      mod_id: 8,
      name: "Darktide Mod Framework",
      summary: "Framework",
      description: null,
      picture_url: "https://example.com/p.png",
      author: "Author",
      version: "1.0",
      downloads: 100,
      endorsements: 10,
      created_timestamp: null,
      updated_timestamp: null,
      domain_name: "warhammer40kdarktide",
      category: "Utilities",
    } satisfies ModDetail;
    expect(enrichCatalogHitFromNexus(sparse, detail)).toMatchObject({
      id: "nexus:warhammer40kdarktide:8",
      name: "Darktide Mod Framework",
      summary: "Framework",
      picture_url: "https://example.com/p.png",
      author: "Author",
      downloads: 100,
      endorsements: 10,
      category: "Utilities",
    });
  });
});

describe("enrichCatalogHitFromThunderstore", () => {
  it("maps package fields like catalog_from_ts", () => {
    const sparse = hit({
      source: "thunderstore",
      id: "thunderstore:lethal-company:BepInEx:BepInExPack",
      name: "BepInEx-BepInExPack",
      community: "lethal-company",
      namespace: "BepInEx",
      package_name: "BepInExPack",
    });
    const detail = {
      community: "lethal-company",
      namespace: "BepInEx",
      name: "BepInExPack",
      full_name: "BepInEx-BepInExPack",
      package_url: "https://thunderstore.io/c/lethal-company/p/BepInEx/BepInExPack/",
      uuid4: "u",
      rating_score: 5,
      is_deprecated: false,
      has_nsfw_content: false,
      categories: ["Mods"],
      description: "Pack",
      icon_url: "https://example.com/icon.png",
      downloads: 50,
      versions: [],
      latest_version: "5.0.0",
    } satisfies TsPackageDetail;
    expect(enrichCatalogHitFromThunderstore(sparse, detail)).toMatchObject({
      name: "BepInExPack",
      author: "BepInEx",
      picture_url: "https://example.com/icon.png",
      downloads: 50,
      package_url: detail.package_url,
      latest_version: "5.0.0",
    });
  });
});

describe("enrichCatalogHitFromModio", () => {
  it("replaces placeholder name", () => {
    const sparse = hit({
      source: "modio",
      id: "modio:2248:99",
      name: "Mod 99",
      modio_game_id: 2248,
      modio_mod_id: 99,
    });
    const detail = {
      game_id: 2248,
      mod_id: 99,
      name: "Cool Mod",
      name_id: "cool-mod",
      summary: "Summary",
      description: null,
      picture_url: null,
      author: "Me",
      downloads: 3,
      profile_url: "https://mod.io/g/2248/m/99",
      tags: ["tag"],
      has_dependencies: false,
      primary_file_id: 1,
    } satisfies ModioModDetail;
    expect(enrichCatalogHitFromModio(sparse, detail).name).toBe("Cool Mod");
  });
});
