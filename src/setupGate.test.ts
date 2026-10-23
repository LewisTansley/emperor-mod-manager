import { describe, expect, it } from "vitest";
import { setupCanFinish } from "./setupGate";

describe("setupCanFinish", () => {
  it("enables finish only when the API key and website sign-in are both done", () => {
    expect(setupCanFinish(true, true)).toBe(true);
    expect(setupCanFinish(true, false)).toBe(false);
    expect(setupCanFinish(false, true)).toBe(false);
    expect(setupCanFinish(false, false)).toBe(false);
  });
});
