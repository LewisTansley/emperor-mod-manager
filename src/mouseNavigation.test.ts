import { describe, expect, it, vi } from "vitest";

import {
  attachMouseNavigationListeners,
  isMouseBack,
  isMouseForward,
  mouseNavDirection,
} from "./mouseNavigation";

describe("mouseNavigation", () => {
  it("detects back and forward buttons", () => {
    expect(isMouseBack({ button: 3 } as MouseEvent)).toBe(true);
    expect(isMouseBack({ button: 0 } as MouseEvent)).toBe(false);
    expect(isMouseForward({ button: 4 } as MouseEvent)).toBe(true);
    expect(isMouseForward({ button: 3 } as MouseEvent)).toBe(false);
  });

  it("maps direction from button id", () => {
    expect(mouseNavDirection({ button: 3 } as MouseEvent)).toBe("back");
    expect(mouseNavDirection({ button: 4 } as MouseEvent)).toBe("forward");
    expect(mouseNavDirection({ button: 0 } as MouseEvent)).toBeNull();
  });

  it("invokes handler for auxclick back/forward with preventDefault", () => {
    const onNav = vi.fn();
    const detach = attachMouseNavigationListeners(onNav);
    const event = new MouseEvent("auxclick", { button: 3, bubbles: true, cancelable: true });
    const prevented = !document.dispatchEvent(event);
    expect(prevented).toBe(true);
    expect(onNav).toHaveBeenCalledWith("back", expect.any(MouseEvent));
    detach();
  });
});
