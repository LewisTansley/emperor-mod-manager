import { createRoot, type Root } from "react-dom/client";
import { act } from "react";
import { describe, expect, it } from "vitest";

import { MissingRequirementsStrip, RequirementsDialog } from "./RequirementsDialog";
import { renderToStaticMarkup } from "react-dom/server";
import type { MissingRequirements, NexusRequirementDownload } from "./types";

const sample: MissingRequirements = {
  nexus: [
    {
      domain: "skyrimspecialedition",
      mod_id: 42,
      file_id: 7,
      name: "Address Library",
      version: "11",
      notes: "SKSE plugin",
      url: "https://www.nexusmods.com/skyrimspecialedition/mods/42",
      required_by: ["1_2"],
      required_by_names: ["My Mod"],
    },
  ],
  external: [
    {
      name: "SKSE",
      url: "https://skse.silverlock.org/",
      notes: null,
      required_by: ["1_2"],
      required_by_names: ["My Mod"],
    },
  ],
  dlc: [
    {
      name: "Far Harbor",
      notes: null,
      required_by: ["1_2"],
      required_by_names: ["My Mod"],
    },
  ],
  error: null,
};

describe("MissingRequirementsStrip", () => {
  it("lists missing requirements and offers review", () => {
    let opened = false;
    const html = renderToStaticMarkup(
      <MissingRequirementsStrip
        requirements={sample}
        onReview={() => {
          opened = true;
        }}
      />,
    );
    expect(html).toContain("3 missing requirements");
    expect(html).toContain("Address Library");
    expect(html).toContain("SKSE");
    expect(html).toContain("Far Harbor");
    expect(html).toContain("Review");
    expect(opened).toBe(false);
  });
});

describe("RequirementsDialog", () => {
  it("queues only the Nexus requirements the user checks", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root: Root = createRoot(host);
    const chosen: NexusRequirementDownload[][] = [];
    await act(async () => {
      root.render(
        <RequirementsDialog
          requirements={sample}
          isPremium
          busy={false}
          onClose={() => {}}
          onAddSelected={(rows) => {
            chosen.push(rows);
          }}
        />,
      );
    });
    const download = document.querySelector(
      'input[aria-label="Download Address Library"]',
    ) as HTMLInputElement;
    expect(download.checked).toBe(false);
    await act(async () => {
      download.click();
    });
    const button = [...document.querySelectorAll("button")].find((el) =>
      el.textContent?.includes("Download selected"),
    ) as HTMLButtonElement;
    await act(async () => {
      button.click();
    });
    expect(chosen).toHaveLength(1);
    expect(chosen[0][0].mod_id).toBe(42);
    expect(document.body.textContent).toContain("SKSE");
    expect(document.body.textContent).toContain("Outside Nexus");
    expect(document.body.textContent).toContain("Far Harbor");
    await act(async () => {
      root.unmount();
    });
    host.remove();
  });
});
