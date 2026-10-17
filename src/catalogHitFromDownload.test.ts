import { describe, expect, it } from "vitest";
import { catalogHitFromDownload, catalogHitFromStaged } from "./App";
import type { CatalogHit, DownloadItem, DownloadPageTarget, ManagedGame, StagedMod } from "./types";

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

function download(label: string, page: DownloadPageTarget | null): DownloadItem {
  return {
    id: "dl",
    label,
    status: "done",
    error: null,
    bytes_downloaded: 0,
    bytes_total: null,
    speed_bps: 0,
    batch_id: null,
    page,
  };
}

function identity(hit: CatalogHit | null) {
  if (!hit) return null;
  const { latest_version: _version, ...rest } = hit;
  return rest;
}

describe("catalogHitFromDownload", () => {
  it("builds the same Nexus hit the library uses", () => {
    const fromLibrary = catalogHitFromStaged(
      staged({
        id: "n1",
        name: "SkyUI",
        source: "nexus",
        domain: "skyrimspecialedition",
        nexus_mod_id: 12604,
      }),
      game(),
    );
    const fromDownload = catalogHitFromDownload(
      download("SkyUI", {
        source: "nexus",
        game_id: "g",
        domain: "skyrimspecialedition",
        mod_id: 12604,
      }),
    );
    expect(identity(fromDownload)).toEqual(identity(fromLibrary));
  });

  it("builds the same Thunderstore hit the library uses", () => {
    const fromLibrary = catalogHitFromStaged(
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
    const fromDownload = catalogHitFromDownload(
      download("BepInEx", {
        source: "thunderstore",
        game_id: "g",
        community: "valheim",
        namespace: "denikson",
        package_name: "BepInExPack_Valheim",
      }),
    );
    expect(identity(fromDownload)).toEqual(identity(fromLibrary));
  });

  it("builds the same mod.io hit the library uses", () => {
    const fromLibrary = catalogHitFromStaged(
      staged({
        id: "m1",
        name: "Better Maps",
        source: "modio",
        modio_game_id: 4242,
        modio_mod_id: 99,
      }),
      game(),
    );
    const fromDownload = catalogHitFromDownload(
      download("Better Maps", {
        source: "modio",
        game_id: "g",
        modio_game_id: 4242,
        modio_mod_id: 99,
      }),
    );
    expect(identity(fromDownload)).toEqual(identity(fromLibrary));
  });

  it("returns null when the download has no catalog target", () => {
    expect(catalogHitFromDownload(download("Loose files", null))).toBeNull();
  });
});
