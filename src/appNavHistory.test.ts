import { describe, expect, it } from "vitest";

import { AppNavHistory, type NavEntry } from "./appNavHistory";

function entry(tab: NavEntry["tab"], patch: Partial<NavEntry> = {}): NavEntry {
  return {
    tab,
    browseReturnTab: null,
    browseDetail: null,
    libraryDetail: null,
    gameSavedDetailId: null,
    savedDetailId: null,
    ...patch,
  };
}

describe("AppNavHistory", () => {
  it("starts at the initial entry", () => {
    const journal = new AppNavHistory(entry("browse"));
    expect(journal.current()).toEqual(entry("browse"));
    expect(journal.currentIndex).toBe(0);
  });

  it("push truncates forward branch", () => {
    const journal = new AppNavHistory(entry("browse"));
    journal.push(entry("browse", { browseDetail: { kind: "mod", hit: { id: "a" } as never, tab: "info" } }));
    journal.back();
    journal.push(entry("library"));
    expect(journal.length).toBe(2);
    expect(journal.current().tab).toBe("library");
    journal.forward();
    expect(journal.current().tab).toBe("library");
  });

  it("back and forward move between entries", () => {
    const journal = new AppNavHistory(entry("browse"));
    journal.push(entry("browse", { savedDetailId: "c1" }));
    expect(journal.currentIndex).toBe(1);
    const prev = journal.back();
    expect(prev?.savedDetailId).toBeNull();
    const again = journal.forward();
    expect(again?.savedDetailId).toBe("c1");
  });

  it("reset collapses history", () => {
    const journal = new AppNavHistory(entry("browse"));
    journal.push(entry("library", { libraryDetail: { gameId: "g1", tab: "mods" } }));
    journal.reset(entry("downloads"));
    expect(journal.length).toBe(1);
    expect(journal.current().tab).toBe("downloads");
    expect(journal.back()).toBeNull();
  });

  it("back works after push from library home (openGame must push, not reset)", () => {
    const journal = new AppNavHistory(entry("library"));
    journal.push(
      entry("library", { libraryDetail: { gameId: "g1", tab: "mods" } }),
    );
    expect(journal.currentIndex).toBe(1);
    const previous = journal.back();
    expect(previous).toEqual(entry("library"));
    expect(previous?.libraryDetail).toBeNull();
  });
});
