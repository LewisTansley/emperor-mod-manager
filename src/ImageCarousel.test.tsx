import { createRoot, type Root } from "react-dom/client";
import { act, type ReactElement } from "react";
import { describe, expect, it } from "vitest";

import { ImageCarousel } from "./ImageCarousel";

async function render(ui: ReactElement) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root: Root = createRoot(host);
  await act(async () => {
    root.render(ui);
  });
  return {
    host,
    async unmount() {
      await act(async () => root.unmount());
      host.remove();
    },
  };
}

const slides = [
  { url: "https://example.com/one.png", caption: "First" },
  { url: "https://example.com/two.jpg", caption: "Second" },
];

describe("ImageCarousel", () => {
  it("renders nothing when there are no slides", async () => {
    const view = await render(<ImageCarousel slides={[]} />);
    expect(view.host.querySelector(".detail-carousel")).toBeNull();
    await view.unmount();
  });

  it("shows a single image without controls", async () => {
    const view = await render(<ImageCarousel slides={[slides[0]]} />);
    const img = view.host.querySelector("img");
    expect(img?.getAttribute("src")).toBe(slides[0].url);
    expect(view.host.textContent).toContain("First");
    expect(view.host.querySelector("button")).toBeNull();
    await view.unmount();
  });

  it("steps through gallery images", async () => {
    const view = await render(<ImageCarousel slides={slides} />);
    const img = () => view.host.querySelector("img");
    expect(img()?.getAttribute("src")).toBe(slides[0].url);
    expect(view.host.textContent).toContain("1 / 2");

    const next = view.host.querySelector<HTMLButtonElement>(
      'button[aria-label="Next image"]',
    );
    await act(async () => {
      next?.click();
    });
    expect(img()?.getAttribute("src")).toBe(slides[1].url);
    expect(view.host.textContent).toContain("Second");
    expect(view.host.textContent).toContain("2 / 2");

    const region = view.host.querySelector<HTMLElement>(".detail-carousel");
    await act(async () => {
      region?.dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }),
      );
    });
    expect(img()?.getAttribute("src")).toBe(slides[0].url);
    await view.unmount();
  });
});
