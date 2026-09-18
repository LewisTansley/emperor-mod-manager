import { useEffect, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";

import { api } from "./api";
import { Modal } from "./Modal";
import type { ModOption, ModOptionSelection, ModOptionsView, StagedMod } from "./types";

type Props = {
  gameId: string;
  mod: StagedMod;
  onClose: () => void;
  onSaved: (selection: ModOptionSelection) => void;
};

function OptionImage({ path, alt }: { path: string | null; alt: string }) {
  if (!path) return null;
  return <img className="mod-option-image" src={convertFileSrc(path)} alt={alt} loading="lazy" />;
}

export function ModOptionsDialog({ gameId, mod, onClose, onSaved }: Props) {
  const [view, setView] = useState<ModOptionsView | null>(null);
  const [selection, setSelection] = useState<ModOptionSelection | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    api
      .getModOptions(gameId, mod.id)
      .then((next) => {
        if (cancelled) return;
        setView(next);
        setSelection(next?.selection ?? null);
        setError(next ? null : "This mod no longer declares any options.");
      })
      .catch((e) => !cancelled && setError(String(e)))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  }, [gameId, mod.id]);

  const isEnabled = (id: string) => selection?.enabled_options.includes(id) ?? false;

  const toggleOption = (option: ModOption, enabled: boolean) => {
    setSelection((prev) => {
      if (!prev) return prev;
      const enabled_options = enabled
        ? [...prev.enabled_options, option.id]
        : prev.enabled_options.filter((id) => id !== option.id);
      const sub_choice = { ...prev.sub_choice };
      if (enabled) {
        // A group with variants needs one picked, or the option contributes nothing.
        if (option.sub_options.length > 0 && !sub_choice[option.id]) {
          sub_choice[option.id] = option.sub_options[0].id;
        }
      } else {
        delete sub_choice[option.id];
      }
      return { enabled_options, sub_choice };
    });
  };

  const chooseSub = (optionId: string, subId: string) => {
    setSelection((prev) =>
      prev ? { ...prev, sub_choice: { ...prev.sub_choice, [optionId]: subId } } : prev,
    );
  };

  const save = async () => {
    if (!selection) return;
    setSaving(true);
    setError(null);
    try {
      const saved = await api.setModOptions(gameId, mod.id, selection);
      onSaved(saved);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const footer = (
    <>
      <small className="field-hint">Deploy again to apply these options.</small>
      <div className="modal-foot-actions">
        <button type="button" className="linkish" onClick={onClose} disabled={saving}>
          Cancel
        </button>
        <button type="button" onClick={save} disabled={saving || !selection}>
          {saving ? "Saving…" : "Save options"}
        </button>
      </div>
    </>
  );

  return (
    <Modal
      title={mod.name}
      subtitle={view?.set.description ?? "Mod options"}
      onClose={onClose}
      footer={view && selection ? footer : undefined}
    >
      {error ? <p className="error">{error}</p> : null}
      {loading ? <p>Reading options…</p> : null}
      {view && selection ? (
        <ul className="mod-option-list">
          {view.set.options.map((option) => {
            const enabled = isEnabled(option.id);
            return (
              <li key={option.id} className={enabled ? undefined : "mod-option-off"}>
                <label className="check">
                  <input
                    type="checkbox"
                    checked={enabled}
                    onChange={(e) => toggleOption(option, e.target.checked)}
                  />
                  <OptionImage path={option.image} alt={option.name} />
                  <div>
                    <strong>{option.name}</strong>
                    {option.description ? <small>{option.description}</small> : null}
                  </div>
                </label>
                {enabled && option.sub_options.length > 0 ? (
                  <ul className="mod-suboption-list">
                    {option.sub_options.map((sub) => (
                      <li key={sub.id}>
                        <label className="check">
                          <input
                            type="radio"
                            name={`sub-${option.id}`}
                            checked={selection.sub_choice[option.id] === sub.id}
                            onChange={() => chooseSub(option.id, sub.id)}
                          />
                          <OptionImage path={sub.image} alt={sub.name} />
                          <div>
                            <strong>{sub.name}</strong>
                            {sub.description ? <small>{sub.description}</small> : null}
                          </div>
                        </label>
                      </li>
                    ))}
                  </ul>
                ) : null}
              </li>
            );
          })}
        </ul>
      ) : null}
    </Modal>
  );
}
