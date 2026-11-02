import { describe, expect, it } from "vitest";

import {
  browsePageCount,
  estimateGridRowHeight,
  gridCardWidth,
  gridColumnCount,
} from "./browseVirtual";

describe("gridColumnCount", () => {
  it("matches auto-fill minmax tracks", () => {
    expect(gridColumnCount(800, 160, 13.6)).toBe(4);
  });

  it("fits an exact number of tracks and drops one when a pixel short", () => {
    expect(gridColumnCount(512, 160, 16)).toBe(3);
    expect(gridColumnCount(511, 160, 16)).toBe(2);
  });

  it("returns one column when the container is narrower than a card", () => {
    expect(gridColumnCount(100, 160, 16)).toBe(1);
  });

  it("returns one column for a non-positive width or minimum", () => {
    expect(gridColumnCount(0, 160, 16)).toBe(1);
    expect(gridColumnCount(-20, 160, 16)).toBe(1);
    expect(gridColumnCount(400, 0, 16)).toBe(1);
  });

  it("ignores a negative gap", () => {
    expect(gridColumnCount(500, 160, -5)).toBe(3);
  });
});

describe("gridCardWidth", () => {
  it("splits the row after column gaps", () => {
    expect(gridCardWidth(512, 3, 16)).toBe(160);
  });

  it("uses the full width for a single column", () => {
    expect(gridCardWidth(100, 1, 16)).toBe(100);
  });
});

describe("browsePageCount", () => {
  it("fills the pane and adds one row past the fold", () => {
    expect(browsePageCount(8, 900, 300, 16)).toBe(32);
  });

  it("falls back when the pane has not been measured", () => {
    expect(browsePageCount(8, 0, 300, 16)).toBe(24);
    expect(browsePageCount(0, 900, 300, 16)).toBe(24);
    expect(browsePageCount(8, 900, 0, 16)).toBe(24);
  });

  it("caps a very tall window", () => {
    expect(browsePageCount(20, 8000, 200, 16)).toBe(120);
  });
});

describe("estimateGridRowHeight", () => {
  it("reserves a 2/3 cover plus title and meta", () => {
    const height = estimateGridRowHeight(160, 16);
    expect(height).toBeGreaterThan(160 * 1.5);
    expect(height).toBeLessThan(160 * 1.5 + 120);
  });
});
