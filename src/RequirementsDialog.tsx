import { useMemo, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { Modal } from "./Modal";
import type {
  DlcRequirementNote,
  ExternalRequirementLink,
  IgnoredRequirement,
  MissingRequirements,
  NexusRequirementDownload,
  StagedMod,
} from "./types";

type Props = {
  requirements: MissingRequirements;
  disabled: DisabledRequirementWarning[];
  ignored?: IgnoredRequirement[];
  versions?: GameVersionWarning[];
  isPremium: boolean;
  busy: boolean;
  onClose: () => void;
  onAddSelected: (rows: NexusRequirementDownload[]) => void;
  onEnable: (stagedIds: string[]) => void;
  onDisable: (stagedIds: string[]) => void;
  onIgnore?: (row: IgnoredRequirement) => void;
};

export type DisabledRequirementWarning = {
  stagedId: string;
  name: string;
  requiredByNames: string[];
  requiredByIds: string[];
};

export type GameVersionWarning = {
  stagedId: string;
  name: string;
  message: string;
};

type RequirementParent = { id: string; name: string };

function uniqueIds(ids: string[]): string[] {
  return ids.filter((id, index) => id && ids.indexOf(id) === index);
}

function uniqueParents(parents: RequirementParent[]): RequirementParent[] {
  return parents.filter(
    (parent, index) =>
      parent.id &&
      parent.name &&
      parents.findIndex((other) => other.id === parent.id) === index,
  );
}

/**
 * Disabled staged mods that an enabled mod still requires.
 * Live `enabled` flags win over the snapshot on each Nexus note, so unticking
 * updates the review without another fetch. `depends_on` covers saved links
 * the Nexus graph does not return.
 */
export function disabledRequirementWarnings(
  mods: Pick<StagedMod, "id" | "name" | "enabled" | "depends_on">[],
  requirements: MissingRequirements | null | undefined,
): DisabledRequirementWarning[] {
  const byId = new Map(mods.map((mod) => [mod.id, mod]));
  const order: string[] = [];
  const rows = new Map<string, { name: string; requiredBy: RequirementParent[] }>();

  function add(stagedId: string, name: string, parentRefs: RequirementParent[]) {
    const parents = uniqueParents(parentRefs);
    if (parents.length === 0) return;
    const existing = rows.get(stagedId);
    if (!existing) {
      order.push(stagedId);
      rows.set(stagedId, { name, requiredBy: parents });
      return;
    }
    for (const parent of parents) {
      if (!existing.requiredBy.some((row) => row.id === parent.id)) {
        existing.requiredBy.push(parent);
      }
    }
  }

  for (const note of requirements?.staged ?? []) {
    const live = byId.get(note.staged_id);
    const enabled = live ? live.enabled : note.enabled;
    if (enabled) continue;
    const parents = note.required_by
      .filter((id) => byId.get(id)?.enabled)
      .map((id) => ({ id, name: byId.get(id)?.name ?? "" }));
    add(note.staged_id, live?.name || note.name, parents);
  }

  for (const parent of mods) {
    if (!parent.enabled) continue;
    for (const depId of parent.depends_on ?? []) {
      const dep = byId.get(depId);
      if (!dep || dep.enabled) continue;
      add(dep.id, dep.name, [{ id: parent.id, name: parent.name }]);
    }
  }

  return order.map((id) => {
    const row = rows.get(id)!;
    return {
      stagedId: id,
      name: row.name,
      requiredByNames: row.requiredBy.map((parent) => parent.name),
      requiredByIds: row.requiredBy.map((parent) => parent.id),
    };
  });
}

export function requirementKey(row: NexusRequirementDownload): string {
  return `${row.domain}:${row.mod_id}:${row.file_id}`;
}

export function nexusIgnoreKey(
  row: Pick<NexusRequirementDownload, "domain" | "mod_id">,
): string {
  return `nexus:${row.domain}:${row.mod_id}`;
}

export function externalIgnoreKey(
  row: Pick<ExternalRequirementLink, "name" | "url">,
): string {
  const url = row.url.trim();
  return `external:${url || row.name.trim()}`;
}

export function dlcIgnoreKey(row: Pick<DlcRequirementNote, "name">): string {
  return `dlc:${row.name.trim()}`;
}

export function stagedIgnoreKey(stagedId: string): string {
  return `staged:${stagedId}`;
}

function ignoredKeys(ignored: IgnoredRequirement[]): Set<string> {
  return new Set(ignored.map((row) => row.key));
}

/** Drops requirements the user has chosen to hide. */
export function withoutIgnoredRequirements(
  requirements: MissingRequirements,
  ignored: IgnoredRequirement[],
): MissingRequirements {
  const keys = ignoredKeys(ignored);
  if (keys.size === 0) return requirements;
  return {
    ...requirements,
    nexus: requirements.nexus.filter((row) => !keys.has(nexusIgnoreKey(row))),
    external: requirements.external.filter((row) => !keys.has(externalIgnoreKey(row))),
    dlc: requirements.dlc.filter((row) => !keys.has(dlcIgnoreKey(row))),
  };
}

export function withoutIgnoredDisabled(
  disabled: DisabledRequirementWarning[],
  ignored: IgnoredRequirement[],
): DisabledRequirementWarning[] {
  const keys = ignoredKeys(ignored);
  if (keys.size === 0) return disabled;
  return disabled.filter((row) => !keys.has(stagedIgnoreKey(row.stagedId)));
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

function requirementCount(count: number, label: string): string {
  return `${count} ${label} requirement${count === 1 ? "" : "s"}`;
}

function versionWarningCount(count: number): string {
  return `${count} game version warning${count === 1 ? "" : "s"}`;
}

export function MissingRequirementsStrip(props: {
  requirements: MissingRequirements;
  disabled?: DisabledRequirementWarning[];
  versions?: GameVersionWarning[];
  onReview: () => void;
}) {
  const { requirements, onReview } = props;
  const disabled = props.disabled ?? [];
  const versions = props.versions ?? [];
  const titles = [
    ...requirements.nexus.map((row) => row.name),
    ...requirements.external.map((row) => row.name),
    ...requirements.dlc.map((row) => row.name),
    ...disabled.map((row) => row.name),
    ...versions.map((row) => row.name),
  ];
  const preview = titles.slice(0, 4);
  const extra = titles.length - preview.length;
  const missingCount =
    requirements.nexus.length + requirements.external.length + requirements.dlc.length;
  const summary = [
    missingCount > 0 ? requirementCount(missingCount, "missing") : null,
    disabled.length > 0 ? requirementCount(disabled.length, "disabled") : null,
    versions.length > 0 ? versionWarningCount(versions.length) : null,
  ].filter((line): line is string => Boolean(line));
  return (
    <div className="missing-requirements">
      <div>
        <strong>
          {summary.length === 0 ? "Could not check requirements" : summary.join(", ")}
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

function DisableButton({
  ids,
  busy,
  onDisable,
}: {
  ids: string[];
  busy: boolean;
  onDisable: (stagedIds: string[]) => void;
}) {
  const unique = uniqueIds(ids);
  if (unique.length === 0) return null;
  return (
    <button type="button" disabled={busy} onClick={() => onDisable(unique)}>
      Disable
    </button>
  );
}

function IgnoreButton({
  entry,
  busy,
  onIgnore,
}: {
  entry: IgnoredRequirement | null;
  busy: boolean;
  onIgnore?: (row: IgnoredRequirement) => void;
}) {
  if (!entry || !onIgnore) return null;
  return (
    <button
      type="button"
      disabled={busy}
      aria-label={`Ignore ${entry.name}`}
      onClick={() => onIgnore(entry)}
    >
      Ignore
    </button>
  );
}

function ignoreEntry(key: string, name: string): IgnoredRequirement | null {
  const trimmed = name.trim();
  const identity = key.slice(key.indexOf(":") + 1).trim();
  if (!trimmed || !identity) return null;
  return { key, name: trimmed };
}

export function RequirementsDialog({
  requirements,
  disabled,
  ignored = [],
  versions = [],
  isPremium,
  busy,
  onClose,
  onAddSelected,
  onEnable,
  onDisable,
  onIgnore,
}: Props) {
  const visibleRequirements = useMemo(
    () => withoutIgnoredRequirements(requirements, ignored),
    [requirements, ignored],
  );
  const visibleDisabled = useMemo(
    () => withoutIgnoredDisabled(disabled, ignored),
    [disabled, ignored],
  );
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const nexusKeys = useMemo(
    () => visibleRequirements.nexus.map(requirementKey),
    [visibleRequirements.nexus],
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

  const chosen = visibleRequirements.nexus.filter((row) =>
    selected.has(requirementKey(row)),
  );
  const dependentIds = useMemo(
    () => uniqueIds(visibleDisabled.flatMap((row) => row.requiredByIds)),
    [visibleDisabled],
  );

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
            {visibleRequirements.nexus.length === 0
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
      {visibleRequirements.nexus.length > 0 && (
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
            {visibleRequirements.nexus.map((row) => {
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
                  <IgnoreButton
                    entry={ignoreEntry(nexusIgnoreKey(row), row.name)}
                    busy={busy}
                    onIgnore={onIgnore}
                  />
                  <DisableButton
                    ids={row.required_by}
                    busy={busy}
                    onDisable={onDisable}
                  />
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
      {visibleRequirements.external.length > 0 && (
        <>
          <h3>Outside Nexus</h3>
          <ul className="list requirements-list">
            {visibleRequirements.external.map((row) => {
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
                  <IgnoreButton
                    entry={ignoreEntry(externalIgnoreKey(row), row.name)}
                    busy={busy}
                    onIgnore={onIgnore}
                  />
                  <DisableButton
                    ids={row.required_by}
                    busy={busy}
                    onDisable={onDisable}
                  />
                </li>
              );
            })}
          </ul>
        </>
      )}
      {visibleRequirements.dlc.length > 0 && (
        <>
          <h3>DLC</h3>
          <ul className="list requirements-list">
            {visibleRequirements.dlc.map((row) => {
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
                  <IgnoreButton
                    entry={ignoreEntry(dlcIgnoreKey(row), row.name)}
                    busy={busy}
                    onIgnore={onIgnore}
                  />
                  <DisableButton
                    ids={row.required_by}
                    busy={busy}
                    onDisable={onDisable}
                  />
                </li>
              );
            })}
          </ul>
        </>
      )}
      {visibleDisabled.length > 0 && (
        <>
          <h3>Disabled</h3>
          <p className="note">
            These mods are installed but unticked. Enabled mods require them.
          </p>
          {visibleDisabled.length > 1 && (
            <div className="requirements-select-all">
              <button
                type="button"
                disabled={busy}
                onClick={() => onEnable(visibleDisabled.map((row) => row.stagedId))}
              >
                Enable all
              </button>
              {dependentIds.length > 0 && (
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => onDisable(dependentIds)}
                >
                  Disable all
                </button>
              )}
            </div>
          )}
          <ul className="list requirements-list">
            {visibleDisabled.map((row) => {
              const by = requiredBy(row.requiredByNames);
              return (
                <li key={row.stagedId}>
                  <div>
                    <strong>{row.name}</strong>
                    {by && <small>{by}</small>}
                  </div>
                  <button
                    type="button"
                    disabled={busy}
                    onClick={() => onEnable([row.stagedId])}
                  >
                    Enable
                  </button>
                  <IgnoreButton
                    entry={ignoreEntry(stagedIgnoreKey(row.stagedId), row.name)}
                    busy={busy}
                    onIgnore={onIgnore}
                  />
                  <DisableButton
                    ids={row.requiredByIds}
                    busy={busy}
                    onDisable={onDisable}
                  />
                </li>
              );
            })}
          </ul>
        </>
      )}
      {versions.length > 0 && (
        <details className="requirements-versions" open>
          <summary>Game version ({versions.length})</summary>
          <ul className="list requirements-list">
            {versions.map((row) => (
              <li key={row.stagedId}>
                <div>
                  <strong>{row.name}</strong>
                  <small>{row.message}</small>
                </div>
              </li>
            ))}
          </ul>
        </details>
      )}
      {visibleRequirements.nexus.length +
        visibleRequirements.external.length +
        visibleRequirements.dlc.length +
        visibleDisabled.length +
        versions.length ===
        0 &&
        !requirements.error && <p className="note">No missing requirements.</p>}
    </Modal>
  );
}

export function IgnoredRequirementsDialog({
  ignored,
  busy,
  onClose,
  onUnignore,
}: {
  ignored: IgnoredRequirement[];
  busy: boolean;
  onClose: () => void;
  onUnignore: (key: string) => void;
}) {
  return (
    <Modal
      title="Ignored requirements"
      subtitle="These stay out of the review until you restore them."
      onClose={onClose}
    >
      <ul className="list requirements-list">
        {ignored.map((row) => (
          <li key={row.key}>
            <div>
              <strong>{row.name}</strong>
            </div>
            <button
              type="button"
              disabled={busy}
              aria-label={`Unignore ${row.name}`}
              onClick={() => onUnignore(row.key)}
            >
              Unignore
            </button>
          </li>
        ))}
      </ul>
    </Modal>
  );
}
