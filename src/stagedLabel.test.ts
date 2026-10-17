import { describe, expect, it } from "vitest";
import { chosenFileLabel, stagedModSourceLine } from "./stagedLabel";
import type { StagedMod } from "./types";

function staged(partial: Partial<StagedMod> & Pick<StagedMod, "id" | "name">): StagedMod {
  return {
    nexus_mod_id: 12604,
    nexus_file_id: 35491,
    version: "5.2",
    domain: "skyrimspecialedition",
    staging_path: "/tmp/mod",
    enabled: true,
    order: 0,
    source: "nexus",
    ...partial,
  };
}

describe("stagedModSourceLine", () => {
  it("shows the chosen file name and version", () => {
    expect(
      stagedModSourceLine(
        staged({ id: "1", name: "SkyUI", file_name: "SkyUI 5.2 SE" }),
      ),
    ).toBe("SkyUI 5.2 SE · v5.2");
  });

  it("keeps the source line and version when the file has no name of its own", () => {
    expect(stagedModSourceLine(staged({ id: "1", name: "SkyUI" }))).toBe(
      "#12604 · file 35491 · v5.2",
    );
    expect(
      stagedModSourceLine(
        staged({ id: "1", name: "SkyUI", file_name: "SkyUI" }),
      ),
    ).toBe("#12604 · file 35491 · v5.2");
  });
});

describe("chosenFileLabel", () => {
  it("prefers the chooser title, then the archive name", () => {
    expect(
      chosenFileLabel("SkyUI", { name: "Optional patch", file_name: "skyui.zip" }),
    ).toBe("Optional patch");
    expect(chosenFileLabel("SkyUI", { name: "SkyUI", file_name: "skyui.zip" })).toBe(
      "skyui.zip",
    );
    expect(chosenFileLabel("SkyUI", { name: "SkyUI", file_name: null })).toBeNull();
  });
});
