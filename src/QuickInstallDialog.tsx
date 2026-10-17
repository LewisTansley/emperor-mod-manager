import { useState } from "react";

import { Modal } from "./Modal";
import type { ModFileInfo } from "./types";

type Props = {
  modName: string;
  mains: ModFileInfo[];
  optionals: ModFileInfo[];
  installLabel: string;
  busy: boolean;
  onClose: () => void;
  onInstall: (main: ModFileInfo, extras: ModFileInfo[]) => void;
};

function fileMeta(file: ModFileInfo): string {
  const parts = [
    file.category_name ?? "file",
    file.version ? `v${file.version}` : null,
    file.is_primary ? "primary" : null,
    file.size_kb != null ? `${(file.size_kb / 1024).toFixed(1)} MB` : null,
  ].filter((part): part is string => Boolean(part));
  return parts.join(" · ");
}

function initialMainId(mains: ModFileInfo[]): number | null {
  return (mains.find((file) => file.is_primary) ?? mains[0])?.file_id ?? null;
}

export function QuickInstallDialog({
  modName,
  mains,
  optionals,
  installLabel,
  busy,
  onClose,
  onInstall,
}: Props) {
  const [mainId, setMainId] = useState<number | null>(() => initialMainId(mains));
  const [extraIds, setExtraIds] = useState<Set<number>>(() => new Set());
  const selected = mains.find((file) => file.file_id === mainId) ?? null;
  const extras = optionals.filter((file) => extraIds.has(file.file_id));

  function toggleExtra(fileId: number) {
    setExtraIds((prev) => {
      const next = new Set(prev);
      if (next.has(fileId)) next.delete(fileId);
      else next.add(fileId);
      return next;
    });
  }

  return (
    <Modal
      title={modName}
      subtitle="Choose which main file to install."
      onClose={onClose}
      footer={
        <div className="modal-foot-actions">
          <button type="button" className="linkish" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button
            type="button"
            disabled={busy || !selected}
            onClick={() => selected && onInstall(selected, extras)}
          >
            {installLabel}
          </button>
        </div>
      }
    >
      <ul className="list quick-install-files">
        {mains.map((file) => (
          <li key={file.file_id}>
            <label className="check">
              <input
                type="radio"
                name="quick-install-main"
                checked={file.file_id === mainId}
                onChange={() => setMainId(file.file_id)}
                aria-label={`Choose ${file.name}`}
              />
              <div>
                <strong>{file.name}</strong>
                <small>{fileMeta(file)}</small>
              </div>
            </label>
          </li>
        ))}
      </ul>
      {optionals.length > 0 && (
        <details className="file-archive">
          <summary>Optional extras ({optionals.length})</summary>
          <ul className="list quick-install-files">
            {optionals.map((file) => (
              <li key={file.file_id}>
                <label className="check">
                  <input
                    type="checkbox"
                    checked={extraIds.has(file.file_id)}
                    onChange={() => toggleExtra(file.file_id)}
                    aria-label={`Include ${file.name}`}
                  />
                  <div>
                    <strong>{file.name}</strong>
                    <small>{fileMeta(file)}</small>
                  </div>
                </label>
              </li>
            ))}
          </ul>
        </details>
      )}
    </Modal>
  );
}
