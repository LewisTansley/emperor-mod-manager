import { describe, expect, it } from "vitest";

import { browseGameVersionOptions } from "./gameVersion";
import type { BrowseMeta } from "./types";

function meta(partial: Partial<BrowseMeta>): BrowseMeta {
  return {
    categories: [],
    mod_tags: [],
    collection_tags: [],
    game_versions: [],
    ...partial,
  };
}

describe("browseGameVersionOptions", () => {
  it("uses version-like mod tags on the mods page", () => {
    expect(
      browseGameVersionOptions(
        meta({
          mod_tags: ["Gameplay", "1.6.1170", "AE"],
          game_versions: ["1.5.97"],
        }),
        "mods",
      ),
    ).toEqual(["1.6.1170"]);
  });

  it("falls back to collection game versions when mods have no version tags", () => {
    expect(
      browseGameVersionOptions(
        meta({ mod_tags: ["Gameplay"], game_versions: ["1.6.1170"] }),
        "mods",
      ),
    ).toEqual(["1.6.1170"]);
  });

  it("drops mod versions that are not a line for the active game", () => {
    expect(
      browseGameVersionOptions(
        meta({
          mod_tags: ["Gameplay", "1.0", "1.6.1170", "AE", "2.1"],
          game_versions: ["1.5.97", "1.0"],
        }),
        "mods",
        "skyrimspecialedition",
      ),
    ).toEqual(["1.6.1170", "AE"]);
  });

  it("keeps the collection list on the collections page", () => {
    expect(
      browseGameVersionOptions(
        meta({ mod_tags: ["1.6"], game_versions: ["1.6.1170"] }),
        "collections",
      ),
    ).toEqual(["1.6.1170"]);
  });
});
