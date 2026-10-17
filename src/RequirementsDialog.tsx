import { useMemo, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { Modal } from "./Modal";
import type {
  ExternalRequirementLink,
  MissingRequirements,
  NexusRequirementDownload,
} from "./types";

type Props = {
  requirements: MissingRequirements;
  isPremium: boolean;
  busy: boolean;
  onClose: () => void;
  onAddSelected: (rows: NexusRequirementDownload[]) => void;
};

export function requirementKey(row: NexusRequirementDownload): string {
  return `${row.domain}:${row.mod_id}:${row.file_id}`;
}

export function requirementsNeedAttention(
  requirements: MissingRequirements | null | undefined,
): boolean {
  if (!requirements) return false;
  return (
    requirements.nexus.length +
      requirements.external.length +
      requirements.dlc.length >
      0 || Boolean(requirements.error)
  );
}

function requiredBy(names: string[]): string | null {
  const unique = names.filter((name, i) => name && names.indexOf(name) === i);
  if (unique.length === 0) return null;
  return `Required by ${unique.join(", ")}`;
}

function openExternal(url: string) {
  void openUrl(url).catch((e) => console.error("Failed to open URL", e));
}

function RequirementLink({ row }: { row: Pick<ExternalRequirementLink, "name" | "url"> }) {
  if (!row.url) return <strong>{row.name}</strong>;
  return (
    <button type="button" className="linkish" onClick={() => openExternal(row.url)}>
      {row.name}
    </button>
  );
}

export function MissingRequirementsStrip(props: {
  requirements: MissingRequirements;
  onReview: () => void;
}) {
  const { requirements, onReview } = props;
  const titles = [
    ...requirements.nexus.map((row) => row.name),
    ...requirements.external.map((row) => row.name),
    ...requirements.dlc.map((row) => row.name),
  ];
  const preview = titles.slice(0, 4);
  const extra = titles.length - preview.length;
  const count = titles.length;
  return (
    <div className="missing-requirements">
      <div>
        <strong>
          {count === 0
            ? "Could not check requirements"
            : `${count} missing requirement${count === 1 ? "" : "s"}`}
        </strong>
        {preview.length > 0 && (
          <ul>
            {preview.map((title, index) => (
              <li key={`${title}:${index}`}>{title}</li>
            ))}
            {extra > 0 && <li>and {extra} more</li>}
          </ul>
        )}
        {requirements.error && <p className="note">{requirements.error}</p>}
      </div>
      <button type="button" onClick={onReview}>
        Review
      </button>
    </div>
  );
}

export function RequirementsDialog({
  requirements,
  isPremium,
  busy,
  onClose,
  onAddSelected,
}: Props) {
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const nexusKeys = useMemo(
    () => requirements.nexus.map(requirementKey),
    [requirements.nexus],
  );
  const allSelected = nexusKeys.length > 0 && nexusKeys.every((key) => selected.has(key));

  function toggle(key: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  function toggleAll() {
    setSelected(allSelected ? new Set() : new Set(nexusKeys));
  }

  const chosen = requirements.nexus.filter((row) => selected.has(requirementKey(row)));

  return (
    <Modal
      title="Missing requirements"
      subtitle={
        isPremium
          ? "Choose which Nexus requirements to download."
          : "Choose which Nexus requirements to add to Download Assist."
      }
      onClose={onClose}
      footer={
        <>
          <span className="note">
            {requirements.nexus.length === 0
              ? "No Nexus mods to download."
              : `${chosen.length} selected`}
          </span>
          <div className="modal-foot-actions">
            <button type="button" onClick={onClose}>
              Close
            </button>
            <button
              type="button"
              disabled={busy || chosen.length === 0}
              onClick={() => onAddSelected(chosen)}
            >
              {isPremium ? "Download selected" : "Add selected to Download Assist"}
            </button>
          </div>
        </>
      }
    >
      {requirements.error && <p className="note">{requirements.error}</p>}
      {requirements.nexus.length > 0 && (
        <>
          <div className="requirements-select-all">
            <label className="check">
              <input
                type="checkbox"
                checked={allSelected}
                onChange={toggleAll}
                aria-label="Select all Nexus requirements"
              />
              Select all
            </label>
          </div>
          <ul className="list requirements-list">
            {requirements.nexus.map((row) => {
              const key = requirementKey(row);
              const by = requiredBy(row.required_by_names);
              return (
                <li key={key}>
                  <label className="check">
                    <input
                      type="checkbox"
                      checked={selected.has(key)}
                      onChange={() => toggle(key)}
                      aria-label={`Download ${row.name}`}
                    />
                  </label>
                  <div>
                    <strong>{row.name}</strong>
                    <small>
                      {row.version ? `v${row.version}` : "Nexus mod"}
                      {by ? ` · ${by}` : ""}
                      {row.notes ? ` · ${row.notes}` : ""}
                    </small>
                  </div>
                  {row.url && (
                    <button
                      type="button"
                      className="linkish"
                      onClick={() => openExternal(row.url)}
                    >
                      Nexus
                    </button>
                  )}
                </li>
              );
            })}
          </ul>
        </>
      )}
      {requirements.external.length > 0 && (
        <>
          <h3>Outside Nexus</h3>
          <ul className="list requirements-list">
            {requirements.external.map((row) => {
              const by = requiredBy(row.required_by_names);
              return (
                <li key={`${row.name}:${row.url}:${by ?? ""}`}>
                  <div>
                    <RequirementLink row={row} />
                    <small>
                      {by}
                      {row.notes ? `${by ? " · " : ""}${row.notes}` : ""}
                    </small>
                  </div>
                </li>
              );
            })}
          </ul>
        </>
      )}
      {requirements.dlc.length > 0 && (
        <>
          <h3>DLC</h3>
          <ul className="list requirements-list">
            {requirements.dlc.map((row) => {
              const by = requiredBy(row.required_by_names);
              return (
                <li key={row.name}>
                  <div>
                    <strong>{row.name}</strong>
                    <small>
                      {by}
                      {row.notes ? `${by ? " · " : ""}${row.notes}` : ""}
                    </small>
                  </div>
                </li>
              );
            })}
          </ul>
        </>
      )}
      {requirements.nexus.length + requirements.external.length + requirements.dlc.length ===
        0 &&
        !requirements.error && <p className="note">No missing requirements.</p>}
    </Modal>
  );
}
