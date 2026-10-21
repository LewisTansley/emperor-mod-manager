import { createRoot, type Root } from "react-dom/client";
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { LoadLogDialog } from "./LoadLogDialog";
import type { LoadDiagnosis } from "./types";

const revealItemInDir = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/plugin-opener", () => ({
  revealItemInDir,
}));

const diagnosis: LoadDiagnosis = {
  summary: "BetterUI failed to load.",
  findings: [
    {
      severity: "error",
      title: "BetterUI failed to load.",
      detail: "FileNotFoundException: Could not load BetterUI.dll",
      related_mod: "BetterUI",
    },
  ],
  report: "Emperor Mod Manager 0.0.0-test\nLikely mod: BetterUI\n",
  excerpt_label: "BepInEx LogOutput.log",
  excerpt_path: "/tmp/BepInEx/LogOutput.log",
  excerpt: "[Message: BepInEx] Chainloader startup complete",
};

describe("LoadLogDialog", () => {
  beforeEach(() => {
    revealItemInDir.mockReset();
  });

  it("shows the finding and copies the report", async () => {
    const written: string[] = [];
    const clipboard = navigator.clipboard;
    Object.assign(navigator, {
      clipboard: {
        writeText: async (text: string) => {
          written.push(text);
        },
      },
    });
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root: Root = createRoot(host);
    await act(async () => {
      root.render(
        <LoadLogDialog
          diagnosis={diagnosis}
          gameTitle="Lethal Company"
          onClose={() => {}}
        />,
      );
    });

    expect(document.body.textContent).toContain("BetterUI failed to load.");
    expect(document.body.textContent).toContain("Lethal Company");
    expect(document.body.textContent).toContain("Could not load BetterUI.dll");
    expect(document.body.textContent).toContain("BepInEx LogOutput.log");
    expect(document.body.textContent).toContain("Chainloader startup complete");

    const button = [...document.querySelectorAll("button")].find(
      (el) => el.textContent === "Copy report",
    ) as HTMLButtonElement;
    await act(async () => {
      button.click();
    });
    expect(written).toEqual([diagnosis.report]);
    expect(document.body.textContent).toContain("Copied");

    await act(async () => {
      root.unmount();
    });
    host.remove();
    Object.assign(navigator, { clipboard });
  });

  it("shows a file-manager error when the log cannot be revealed", async () => {
    revealItemInDir.mockRejectedValue(new Error("permission denied"));
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root: Root = createRoot(host);
    await act(async () => {
      root.render(
        <LoadLogDialog diagnosis={diagnosis} gameTitle="Lethal Company" onClose={() => {}} />,
      );
    });

    const button = [...document.querySelectorAll("button")].find(
      (el) => el.textContent === "BepInEx LogOutput.log",
    ) as HTMLButtonElement;
    await act(async () => {
      button.click();
    });

    expect(revealItemInDir).toHaveBeenCalledWith("/tmp/BepInEx/LogOutput.log");
    expect(document.body.textContent).toContain("permission denied");

    await act(async () => {
      root.unmount();
    });
    host.remove();
  });
});
