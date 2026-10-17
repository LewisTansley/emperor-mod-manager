import { describe, expect, it } from "vitest";
import { catalogHitFromStaged, installedCatalogIds } from "./App";
import type { ManagedGame, StagedMod } from "./types";

function game(partial: Partial<ManagedGame> = {}): ManagedGame {
  return {
    id: "g",
    title: "Game",
    nexus_domain: "skyrimspecialedition",
    install_path: "/games/skyrim",
    launcher: "steam",
    plugin_id: "skyrim",
    thunderstore_community: "skyrim",
    ...partial,
  };
}

function staged(partial: Partial<StagedMod> & Pick<StagedMod, "id" | "name">): StagedMod {
  return {
    nexus_mod_id: 0,
    nexus_file_id: 0,
    version: "1.2.3",
    domain: "",
    staging_path: "/tmp/mod",
    enabled: true,
    order: 0,
    ...partial,
  };
}

describe("catalogHitFromStaged", () => {
  it("builds a Nexus hit from the staged domain and mod id", () => {
    const hit = catalogHitFromStaged(
      staged({
        id: "n1",
        name: "SkyUI",
        source: "nexus",
        domain: "skyrimspecialedition",
        nexus_mod_id: 12604,
        nexus_file_id: 99,
      }),
      game(),
    );
    expect(hit).toMatchObject({
      source: "nexus",
      name: "SkyUI",
      mod_id: 12604,
      domain_name: "skyrimspecialedition",
      latest_version: "1.2.3",
    });
  });

  it("falls back to the game Nexus domain", () => {
    const hit = catalogHitFromStaged(
      staged({ id: "n2", name: "SkyUI", source: "nexus", nexus_mod_id: 12604 }),
      game(),
    );
    expect(hit?.domain_name).toBe("skyrimspecialedition");
  });

  it("skips Nexus mods with no mod id", () => {
    expect(
      catalogHitFromStaged(
        staged({ id: "n0", name: "Loose files", source: "nexus", nexus_mod_id: 0 }),
        game(),
      ),
    ).toBeNull();
  });

  it("builds a Thunderstore hit from the staged community", () => {
    const hit = catalogHitFromStaged(
      staged({
        id: "ts1",
        name: "BepInEx",
        source: "thunderstore",
        domain: "valheim",
        ts_namespace: "denikson",
        ts_name: "BepInExPack_Valheim",
      }),
      game({ thunderstore_community: "skyrim" }),
    );
    expect(hit).toMatchObject({
      source: "thunderstore",
      community: "valheim",
      namespace: "denikson",
      package_name: "BepInExPack_Valheim",
      full_name: "denikson-BepInExPack_Valheim",
    });
  });

  it("skips Thunderstore mods missing a package name", () => {
    expect(
      catalogHitFromStaged(
        staged({
          id: "ts0",
          name: "Broken",
          source: "thunderstore",
          domain: "valheim",
          ts_namespace: "denikson",
        }),
        game(),
      ),
    ).toBeNull();
  });

  it("builds a mod.io hit from the stored ids", () => {
    const hit = catalogHitFromStaged(
      staged({
        id: "m1",
        name: "Graphics",
        source: "modio",
        modio_game_id: 306,
        modio_mod_id: 12345,
      }),
      game(),
    );
    expect(hit).toMatchObject({
      source: "modio",
      modio_game_id: 306,
      modio_mod_id: 12345,
      latest_version: "1.2.3",
    });
  });

  it("skips mod.io mods missing an id", () => {
    expect(
      catalogHitFromStaged(
        staged({ id: "m0", name: "Graphics", source: "modio", modio_game_id: 306 }),
        game(),
      ),
    ).toBeNull();
  });
});

describe("installedCatalogIds", () => {
  it("keeps browse catalog ids for staged mods and drops mods with no id", () => {
    const ids = installedCatalogIds(
      [
        staged({
          id: "n1",
          name: "SkyUI",
          source: "nexus",
          domain: "skyrimspecialedition",
          nexus_mod_id: 12604,
          enabled: false,
        }),
        staged({
          id: "ts1",
          name: "BepInEx",
          source: "thunderstore",
          domain: "valheim",
          ts_namespace: "denikson",
          ts_name: "BepInExPack_Valheim",
        }),
        staged({
          id: "m1",
          name: "Graphics",
          source: "modio",
          modio_game_id: 306,
          modio_mod_id: 12345,
        }),
        staged({ id: "n0", name: "Loose files", source: "nexus", nexus_mod_id: 0 }),
      ],
      game(),
    );
    expect([...ids]).toEqual([
      "nexus:skyrimspecialedition:12604",
      "thunderstore:valheim:denikson:BepInExPack_Valheim",
      "modio:306:12345",
    ]);
  });
});
