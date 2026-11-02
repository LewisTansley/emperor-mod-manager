import { createRoot, type Root } from "react-dom/client";
import { act, type ReactElement } from "react";
import { describe, expect, it } from "vitest";

import { UpdateToast } from "./UpdateToast";

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

function buttonNamed(host: HTMLElement, label: string) {
  return [...host.querySelectorAll("button")].find((button) => button.textContent === label);
}

describe("UpdateToast", () => {
  it("offers Update and reports Update and Later", async () => {
    let updated = false;
    let later = false;
    const view = await render(
      <UpdateToast
        show
        version="1.2.3"
        progress={null}
        canInstall
        checking={false}
        installing={false}
        onUpdate={() => {
          updated = true;
        }}
        onOpenRelease={() => {}}
        onLater={() => {
          later = true;
        }}
      />,
    );
    expect(view.host.textContent).toContain("Update available");
    expect(view.host.textContent).toContain("v1.2.3");
    expect(buttonNamed(view.host, "Open release")).toBeUndefined();

    await act(async () => {
      buttonNamed(view.host, "Update")?.click();
    });
    await act(async () => {
      buttonNamed(view.host, "Later")?.click();
    });
    expect(updated).toBe(true);
    expect(later).toBe(true);
    await view.unmount();
  });

  it("offers Open release when this install cannot be replaced", async () => {
    let opened = false;
    const view = await render(
      <UpdateToast
        show
        version="1.2.3"
        progress={null}
        canInstall={false}
        checking={false}
        installing={false}
        onUpdate={() => {}}
        onOpenRelease={() => {
          opened = true;
        }}
        onLater={() => {}}
      />,
    );
    expect(buttonNamed(view.host, "Update")).toBeUndefined();
    await act(async () => {
      buttonNamed(view.host, "Open release")?.click();
    });
    expect(opened).toBe(true);
    await view.unmount();
  });

  it("shows progress and keeps Later disabled while installing", async () => {
    const view = await render(
      <UpdateToast
        show
        version="1.2.3"
        progress="Downloading…"
        canInstall
        checking={false}
        installing
        onUpdate={() => {}}
        onOpenRelease={() => {}}
        onLater={() => {}}
      />,
    );
    expect(view.host.textContent).toContain("Downloading…");
    expect(buttonNamed(view.host, "Updating…")?.disabled).toBe(true);
    expect(buttonNamed(view.host, "Later")?.disabled).toBe(true);
    await view.unmount();
  });
});
