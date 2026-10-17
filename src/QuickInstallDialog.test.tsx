import { createRoot, type Root } from "react-dom/client";
import { act } from "react";
import { describe, expect, it } from "vitest";

import { QuickInstallDialog } from "./QuickInstallDialog";
import type { ModFileInfo } from "./types";

function file(partial: Partial<ModFileInfo> & Pick<ModFileInfo, "file_id" | "name">): ModFileInfo {
  return {
    file_name: null,
    version: null,
    category_name: null,
    size_kb: null,
    uploaded_timestamp: null,
    is_primary: false,
    ...partial,
  };
}

const bepinex = file({
  file_id: 1,
  name: "BepInEx",
  category_name: "MAIN",
  version: "1.0.0",
});
const melon = file({
  file_id: 2,
  name: "MelonLoader",
  category_name: "MAIN",
  version: "3.0.0",
  is_primary: true,
});
const extra = file({
  file_id: 3,
  name: "Optional textures",
  category_name: "OPTIONAL",
});

describe("QuickInstallDialog", () => {
  it("offers each main file and keeps optional extras collapsed", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root: Root = createRoot(host);
    const installed: ModFileInfo[][] = [];
    await act(async () => {
      root.render(
        <QuickInstallDialog
          modName="Cool Mod"
          mains={[bepinex, melon]}
          optionals={[extra]}
          installLabel="Install"
          busy={false}
          onClose={() => {}}
          onInstall={(main, extras) => {
            installed.push([main, ...extras]);
          }}
        />,
      );
    });

    expect(document.body.textContent).toContain("BepInEx");
    expect(document.body.textContent).toContain("MelonLoader");
    const archived = document.querySelector("details.file-archive");
    expect(archived?.querySelector("summary")?.textContent).toBe("Optional extras (1)");
    expect(archived?.hasAttribute("open")).toBe(false);
    expect(archived?.textContent).toContain("Optional textures");

    const primary = document.querySelector(
      'input[aria-label="Choose MelonLoader"]',
    ) as HTMLInputElement;
    expect(primary.checked).toBe(true);

    const include = document.querySelector(
      'input[aria-label="Include Optional textures"]',
    ) as HTMLInputElement;
    const chooseBep = document.querySelector(
      'input[aria-label="Choose BepInEx"]',
    ) as HTMLInputElement;
    await act(async () => {
      chooseBep.click();
      include.click();
    });
    const button = [...document.querySelectorAll("button")].find(
      (el) => el.textContent === "Install",
    ) as HTMLButtonElement;
    await act(async () => {
      button.click();
    });
    expect(installed).toEqual([[bepinex, extra]]);

    await act(async () => {
      root.unmount();
    });
    host.remove();
  });
});