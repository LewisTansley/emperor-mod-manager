import { createRoot, type Root } from "react-dom/client";
import { act, type ReactElement } from "react";
import { describe, expect, it } from "vitest";

import { StatusBanner, deployStatusNotice, hiddenBannerDetails } from "./StatusBanner";
import type { DeployResult, MissingRequirements } from "./types";

const requirements: MissingRequirements = {
  nexus: [
    {
      domain: "skyrimspecialedition",
      mod_id: 42,
      file_id: 7,
      name: "Address Library",
      version: "11",
      notes: null,
      url: "https://www.nexusmods.com/skyrimspecialedition/mods/42",
      required_by: ["1_2"],
      required_by_names: ["My Mod"],
    },
  ],
  external: [],
  dlc: [],
  error: null,
};

function deployResult(overrides: Partial<DeployResult> = {}): DeployResult {
  return {
    file_count: 4,
    enabled_mods: 2,
    warnings: [],
    ...overrides,
  };
}

describe("hiddenBannerDetails", () => {
  it("hides a detail that is already the whole message", () => {
    expect(hiddenBannerDetails("Only this", ["Only this"])).toEqual([]);
  });

  it("keeps every warning when the message is a summary", () => {
    expect(hiddenBannerDetails("Deployed 2 mods.", ["Skipped a", "Skipped b"])).toEqual([
      "Skipped a",
      "Skipped b",
    ]);
  });
});

describe("deployStatusNotice", () => {
  it("keeps every warning and does not open the requirements modal until asked", () => {
    const opened: MissingRequirements[] = [];
    const notice = deployStatusNotice(
      deployResult({
        warnings: ["Skipped A: missing master B.", "Skipped C: missing master D."],
        plugin_skips: [
          { plugin: "A.esp", missing_master: "B.esm" },
          { plugin: "C.esp", missing_master: "D.esm" },
        ],
        missing_requirements: requirements,
      }),
      (payload) => opened.push(payload),
    );
    expect(notice.kind).toBe("warn");
    expect(notice.message).toBe("Deployed 4 files from 2 mods.");
    expect(notice.message).not.toContain("Skipped");
    expect(notice.details).toEqual([
      "Skipped A: missing master B.",
      "Skipped C: missing master D.",
    ]);
    expect(opened).toEqual([]);
    notice.action?.run();
    expect(opened).toEqual([requirements]);
    expect(notice.action?.label).toBe("Review requirements");
  });

  it("leaves the warning readable when there is nothing to queue or link", () => {
    const notice = deployStatusNotice(
      deployResult({
        warnings: ["Skipped A: missing master B."],
        plugin_skips: [{ plugin: "A.esp", missing_master: "B.esm" }],
        missing_requirements: { nexus: [], external: [], dlc: [], error: null },
      }),
      () => {
        throw new Error("modal should stay closed");
      },
    );
    expect(notice.action).toBeUndefined();
    expect(notice.details).toEqual(["Skipped A: missing master B."]);
  });

  it("does not offer review when deploy did not skip a master", () => {
    const notice = deployStatusNotice(
      deployResult({
        warnings: ["Something else."],
        missing_requirements: requirements,
      }),
      () => {
        throw new Error("modal should stay closed");
      },
    );
    expect(notice.action).toBeUndefined();
  });

  it("uses the enabled-mod summary when there are no warnings", () => {
    const notice = deployStatusNotice(deployResult(), () => {});
    expect(notice).toEqual({
      kind: "ok",
      message: "Deployed 4 files from 2 enabled mods.",
    });
  });
});

describe("StatusBanner", () => {
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

  it("shows the action and the full details without expanding first", async () => {
    let reviewed = false;
    let dismissed = false;
    const view = await render(
      <StatusBanner
        kind="warn"
        message="Deployed 4 files from 2 mods."
        details={["Skipped A: missing master B.", "Skipped C: missing master D."]}
        action={{
          label: "Review requirements",
          run: () => {
            reviewed = true;
          },
        }}
        onDismiss={() => {
          dismissed = true;
        }}
      />,
    );
    expect(view.host.textContent).toContain("Review requirements");
    expect(view.host.textContent).not.toContain("Skipped A");
    const review = [...view.host.querySelectorAll("button")].find(
      (button) => button.textContent === "Review requirements",
    );
    await act(async () => {
      review?.click();
    });
    expect(reviewed).toBe(true);

    const more = [...view.host.querySelectorAll("button")].find(
      (button) => button.textContent === "Show more",
    );
    await act(async () => {
      more?.click();
    });
    expect(view.host.textContent).toContain("Skipped A: missing master B.");
    expect(view.host.textContent).toContain("Skipped C: missing master D.");
    expect(view.host.textContent).toContain("Review requirements");

    const dismiss = [...view.host.querySelectorAll("button")].find(
      (button) => button.textContent === "dismiss",
    );
    await act(async () => {
      dismiss?.click();
    });
    expect(dismissed).toBe(true);
    await view.unmount();
  });

  it("omits show more when the message is the whole notice", async () => {
    const view = await render(
      <StatusBanner kind="error" message="Install path is required." onDismiss={() => {}} />,
    );
    expect(view.host.textContent).not.toContain("Show more");
    expect(view.host.textContent).toContain("dismiss");
    await view.unmount();
  });
});
