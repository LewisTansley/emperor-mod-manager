import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { ModFileInfo } from "./types";
import { NexusModFileList, pickMainModFile } from "./App";

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

describe("NexusModFileList", () => {
  it("keeps current files visible and collapses archived and old versions", () => {
    const html = renderToStaticMarkup(
      <NexusModFileList
        files={[
          file({ file_id: 1, name: "Current pack", category_name: "MAIN", is_primary: true }),
          file({ file_id: 2, name: "Previous build", category_name: "OLD_VERSION" }),
          file({ file_id: 3, name: "Retired pack", category_name: "Archived" }),
        ]}
        installLabel="Install"
        onInstall={() => {}}
      />,
    );
    const root = document.createElement("div");
    root.innerHTML = html;
    const visible = [...root.querySelectorAll(":scope > ul strong")].map(
      (node) => node.textContent,
    );
    const archived = root.querySelector("details");

    expect(visible).toEqual(["Current pack"]);
    expect(archived?.querySelector("summary")?.textContent).toBe(
      "Archived and old versions (2)",
    );
    expect(archived?.hasAttribute("open")).toBe(false);
    expect(
      [...(archived?.querySelectorAll("strong") ?? [])].map((node) => node.textContent),
    ).toEqual(["Previous build", "Retired pack"]);
  });

  it("shows the empty state only when there are no files", () => {
    const html = renderToStaticMarkup(
      <NexusModFileList files={[]} installLabel="Install" onInstall={() => {}} />,
    );
    expect(html).toContain("No files found for this mod.");
  });
});

describe("pickMainModFile", () => {
  it("prefers the primary file, then MAIN, and skips archived files", () => {
    const primary = file({
      file_id: 1,
      name: "Primary",
      category_name: "OPTIONAL",
      is_primary: true,
    });
    const main = file({ file_id: 2, name: "Main", category_name: "MAIN" });
    const old = file({
      file_id: 3,
      name: "Old primary",
      category_name: "OLD_VERSION",
      is_primary: true,
    });

    expect(pickMainModFile([old, main, primary])?.file_id).toBe(1);
    expect(pickMainModFile([old, main])?.file_id).toBe(2);
    expect(pickMainModFile([old])).toBeNull();
  });
});
