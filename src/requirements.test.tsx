import { createRoot, type Root } from "react-dom/client";
import { act } from "react";
import { describe, expect, it } from "vitest";

import {
  disabledRequirementWarnings,
  dlcIgnoreKey,
  externalIgnoreKey,
  IgnoredRequirementsDialog,
  MissingRequirementsStrip,
  nexusIgnoreKey,
  RequirementsDialog,
  stagedIgnoreKey,
  withoutIgnoredDisabled,
  withoutIgnoredRequirements,
} from "./RequirementsDialog";
import { renderToStaticMarkup } from "react-dom/server";
import { useState } from "react";
import type { IgnoredRequirement, MissingRequirements, NexusRequirementDownload } from "./types";

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

  it("counts disabled requirements beside missing ones", () => {
    const html = renderToStaticMarkup(
      <MissingRequirementsStrip
        requirements={{ nexus: [], external: [], dlc: [], error: null }}
        disabled={[
          {
            stagedId: "skyui",
            name: "SkyUI",
            requiredByNames: ["My Mod"],
            requiredByIds: ["parent"],
          },
        ]}
        onReview={() => {}}
      />,
    );
    expect(html).toContain("1 disabled requirement");
    expect(html).toContain("SkyUI");
    expect(html).not.toContain("missing requirement");
    expect(html).not.toContain("Could not check requirements");
  });

  it("counts game version warnings", () => {
    const html = renderToStaticMarkup(
      <MissingRequirementsStrip
        requirements={{ nexus: [], external: [], dlc: [], error: null }}
        versions={[
          {
            stagedId: "skyui",
            name: "SkyUI",
            message: "Does not officially support 1.6.1170",
          },
        ]}
        onReview={() => {}}
      />,
    );
    expect(html).toContain("1 game version warning");
    expect(html).toContain("SkyUI");
  });

  it("does not count ignored rows as missing", () => {
    const visible = withoutIgnoredRequirements(sample, [
      { key: nexusIgnoreKey(sample.nexus[0]), name: "Address Library" },
      { key: externalIgnoreKey(sample.external[0]), name: "SKSE" },
      { key: dlcIgnoreKey(sample.dlc[0]), name: "Far Harbor" },
    ]);
    const html = renderToStaticMarkup(
      <MissingRequirementsStrip requirements={visible} onReview={() => {}} />,
    );
    expect(html).not.toContain("missing requirement");
    expect(html).not.toContain("Address Library");
    expect(html).not.toContain("SKSE");
    expect(html).not.toContain("Far Harbor");
  });
});

describe("withoutIgnoredRequirements", () => {
  const ignored: IgnoredRequirement[] = [
    { key: nexusIgnoreKey(sample.nexus[0]), name: "Address Library" },
    { key: externalIgnoreKey(sample.external[0]), name: "SKSE" },
    { key: dlcIgnoreKey(sample.dlc[0]), name: "Far Harbor" },
    { key: stagedIgnoreKey("skyui"), name: "SkyUI" },
  ];

  it("drops each ignored requirement kind and keeps the rest of the payload", () => {
    expect(externalIgnoreKey({ name: "SKSE", url: "  " })).toBe("external:SKSE");
    const visible = withoutIgnoredRequirements(
      {
        ...sample,
        error: "partial",
        nexus: [
          ...sample.nexus,
          {
            ...sample.nexus[0],
            mod_id: 99,
            name: "Still missing",
          },
        ],
      },
      ignored,
    );
    expect(visible.nexus.map((row) => row.name)).toEqual(["Still missing"]);
    expect(visible.external).toEqual([]);
    expect(visible.dlc).toEqual([]);
    expect(visible.error).toBe("partial");
    expect(
      withoutIgnoredDisabled(
        [
          {
            stagedId: "skyui",
            name: "SkyUI",
            requiredByNames: ["My Mod"],
            requiredByIds: ["parent"],
          },
          {
            stagedId: "local",
            name: "Local Dep",
            requiredByNames: ["My Mod"],
            requiredByIds: ["parent"],
          },
        ],
        ignored,
      ).map((row) => row.stagedId),
    ).toEqual(["local"]);
  });
});

describe("disabledRequirementWarnings", () => {
  const mods = [
    { id: "parent", name: "My Mod", enabled: true, depends_on: ["skyui", "local"] },
    { id: "other", name: "Quiet", enabled: false, depends_on: ["skyui"] },
    { id: "skyui", name: "SkyUI", enabled: false, depends_on: [] },
    { id: "library", name: "Address Library", enabled: true, depends_on: [] },
    { id: "local", name: "Local Dep", enabled: false, depends_on: [] },
  ];

  it("keeps disabled staged requirements of enabled mods and live unticks", () => {
    const warnings = disabledRequirementWarnings(mods, {
      nexus: [],
      external: [],
      dlc: [],
      staged: [
        {
          staged_id: "skyui",
          name: "SkyUI snapshot",
          enabled: true,
          required_by: ["parent", "other"],
          required_by_names: ["My Mod", "Quiet"],
        },
        {
          staged_id: "library",
          name: "Address Library",
          enabled: true,
          required_by: ["parent"],
          required_by_names: ["My Mod"],
        },
      ],
    });
    expect(warnings).toEqual([
      {
        stagedId: "skyui",
        name: "SkyUI",
        requiredByNames: ["My Mod"],
        requiredByIds: ["parent"],
      },
      {
        stagedId: "local",
        name: "Local Dep",
        requiredByNames: ["My Mod"],
        requiredByIds: ["parent"],
      },
    ]);
  });

  it("drops a disabled mod that only a disabled mod requires", () => {
    const warnings = disabledRequirementWarnings(
      [
        { id: "other", name: "Quiet", enabled: false, depends_on: ["skyui"] },
        { id: "skyui", name: "SkyUI", enabled: false, depends_on: [] },
      ],
      {
        nexus: [],
        external: [],
        dlc: [],
        staged: [
          {
            staged_id: "skyui",
            name: "SkyUI",
            enabled: false,
            required_by: ["other"],
            required_by_names: ["Quiet"],
          },
        ],
      },
    );
    expect(warnings).toEqual([]);
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
          disabled={[]}
          isPremium
          busy={false}
          onClose={() => {}}
          onAddSelected={(rows) => {
            chosen.push(rows);
          }}
          onEnable={() => {}}
          onDisable={() => {}}
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

  it("enables one disabled requirement or all of them", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root: Root = createRoot(host);
    const enabled: string[][] = [];
    const disabled = [
      {
        stagedId: "skyui",
        name: "SkyUI",
        requiredByNames: ["My Mod"],
        requiredByIds: ["parent"],
      },
      {
        stagedId: "local",
        name: "Local Dep",
        requiredByNames: ["My Mod", "Other Mod"],
        requiredByIds: ["parent", "other"],
      },
    ];
    await act(async () => {
      root.render(
        <RequirementsDialog
          requirements={{ nexus: [], external: [], dlc: [], error: null }}
          disabled={disabled}
          isPremium
          busy={false}
          onClose={() => {}}
          onAddSelected={() => {}}
          onEnable={(ids) => {
            enabled.push(ids);
          }}
          onDisable={() => {}}
        />,
      );
    });
    expect(document.body.textContent).toContain("Required by My Mod");
    const enableSky = [...document.querySelectorAll("button")].find(
      (el) => el.textContent === "Enable",
    ) as HTMLButtonElement;
    await act(async () => {
      enableSky.click();
    });
    const enableAll = [...document.querySelectorAll("button")].find(
      (el) => el.textContent === "Enable all",
    ) as HTMLButtonElement;
    await act(async () => {
      enableAll.click();
    });
    expect(enabled).toEqual([["skyui"], ["skyui", "local"]]);
    await act(async () => {
      root.unmount();
    });
    host.remove();
  });

  it("lists game version warnings in the review", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root: Root = createRoot(host);
    await act(async () => {
      root.render(
        <RequirementsDialog
          requirements={{ nexus: [], external: [], dlc: [], error: null }}
          disabled={[]}
          versions={[
            {
              stagedId: "skyui",
              name: "SkyUI",
              message: "Does not officially support 1.6.1170",
            },
          ]}
          isPremium
          busy={false}
          onClose={() => {}}
          onAddSelected={() => {}}
          onEnable={() => {}}
          onDisable={() => {}}
        />,
      );
    });
    const section = document.querySelector("details.requirements-versions") as HTMLDetailsElement;
    expect(section.open).toBe(true);
    expect(section.querySelector("summary")?.textContent).toBe("Game version (1)");
    expect(document.body.textContent).toContain("Game version");
    expect(document.body.textContent).toContain("Does not officially support 1.6.1170");
    expect(document.body.textContent).not.toContain("No missing requirements");
    await act(async () => {
      root.unmount();
    });
    host.remove();
  });

  it("disables the mods that require a missing or disabled requirement", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root: Root = createRoot(host);
    const turnedOff: string[][] = [];
    await act(async () => {
      root.render(
        <RequirementsDialog
          requirements={sample}
          disabled={[
            {
              stagedId: "skyui",
              name: "SkyUI",
              requiredByNames: ["My Mod"],
              requiredByIds: ["parent"],
            },
            {
              stagedId: "local",
              name: "Local Dep",
              requiredByNames: ["My Mod", "Other Mod"],
              requiredByIds: ["parent", "other"],
            },
          ]}
          isPremium
          busy={false}
          onClose={() => {}}
          onAddSelected={() => {}}
          onEnable={() => {}}
          onDisable={(ids) => {
            turnedOff.push(ids);
          }}
        />,
      );
    });
    const disableOne = [...document.querySelectorAll("button")].find(
      (el) => el.textContent === "Disable",
    ) as HTMLButtonElement;
    await act(async () => {
      disableOne.click();
    });
    const disableAll = [...document.querySelectorAll("button")].find(
      (el) => el.textContent === "Disable all",
    ) as HTMLButtonElement;
    await act(async () => {
      disableAll.click();
    });
    expect(turnedOff[0]).toEqual(["1_2"]);
    expect(turnedOff[turnedOff.length - 1]).toEqual(["parent", "other"]);
    await act(async () => {
      root.unmount();
    });
    host.remove();
  });

  it("keeps ignored requirements out of the review", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root: Root = createRoot(host);
    const initial: IgnoredRequirement[] = [
      { key: externalIgnoreKey(sample.external[0]), name: "SKSE" },
      { key: dlcIgnoreKey(sample.dlc[0]), name: "Far Harbor" },
      { key: stagedIgnoreKey("skyui"), name: "SkyUI" },
    ];
    function Harness() {
      const [ignored, setIgnored] = useState(initial);
      return (
        <RequirementsDialog
          requirements={sample}
          disabled={[
            {
              stagedId: "skyui",
              name: "SkyUI",
              requiredByNames: ["My Mod"],
              requiredByIds: ["parent"],
            },
          ]}
          ignored={ignored}
          isPremium
          busy={false}
          onClose={() => {}}
          onAddSelected={() => {}}
          onEnable={() => {}}
          onDisable={() => {}}
          onIgnore={(row) => setIgnored((prev) => [...prev, row])}
        />
      );
    }
    await act(async () => {
      root.render(<Harness />);
    });
    expect(document.querySelector('input[aria-label="Download Address Library"]')).not.toBeNull();
    expect(document.body.textContent).not.toContain("Outside Nexus");
    expect(document.body.textContent).not.toContain("DLC");
    expect(document.body.textContent).not.toContain("Disabled");
    expect(document.body.textContent).not.toContain("Far Harbor");
    expect(document.body.textContent).not.toContain("SkyUI");
    expect(document.querySelector('button[aria-label="Unignore SKSE"]')).toBeNull();
    expect(document.querySelector('button[aria-label^="Unignore "]')).toBeNull();
    const ignore = document.querySelector(
      'button[aria-label="Ignore Address Library"]',
    ) as HTMLButtonElement;
    await act(async () => {
      ignore.click();
    });
    expect(document.querySelector('input[aria-label="Download Address Library"]')).toBeNull();
    expect(document.body.textContent).not.toContain("Address Library");
    expect(document.querySelector('button[aria-label^="Unignore "]')).toBeNull();
    expect(document.body.textContent).toContain("No missing requirements");
    await act(async () => {
      root.unmount();
    });
    host.remove();
  });
});

describe("IgnoredRequirementsDialog", () => {
  it("lists ignored requirements and restores one", async () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root: Root = createRoot(host);
    const initial: IgnoredRequirement[] = [
      { key: nexusIgnoreKey(sample.nexus[0]), name: "Address Library" },
      { key: externalIgnoreKey(sample.external[0]), name: "SKSE" },
    ];
    function Harness() {
      const [ignored, setIgnored] = useState(initial);
      return (
        <IgnoredRequirementsDialog
          ignored={ignored}
          busy={false}
          onClose={() => {}}
          onUnignore={(key) => setIgnored((prev) => prev.filter((row) => row.key !== key))}
        />
      );
    }
    await act(async () => {
      root.render(<Harness />);
    });
    expect(document.body.textContent).toContain("Ignored requirements");
    expect(document.body.textContent).toContain(
      "These stay out of the review until you restore them.",
    );
    expect(document.querySelector('button[aria-label="Unignore Address Library"]')).not.toBeNull();
    expect(document.querySelector('button[aria-label="Unignore SKSE"]')).not.toBeNull();
    const restore = document.querySelector(
      'button[aria-label="Unignore Address Library"]',
    ) as HTMLButtonElement;
    await act(async () => {
      restore.click();
    });
    expect(document.querySelector('button[aria-label="Unignore Address Library"]')).toBeNull();
    expect(document.querySelector('button[aria-label="Unignore SKSE"]')).not.toBeNull();
    await act(async () => {
      root.unmount();
    });
    host.remove();
  });
});
