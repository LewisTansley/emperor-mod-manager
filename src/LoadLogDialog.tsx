import { useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

import { Modal } from "./Modal";
import type { LoadDiagnosis } from "./types";

export function LoadLogDialog({
  diagnosis,
  gameTitle,
  onClose,
}: {
  diagnosis: LoadDiagnosis;
  gameTitle?: string;
  onClose: () => void;
}) {
  const [copied, setCopied] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);
  const logLabel = diagnosis.excerpt_label || diagnosis.excerpt_path;

  return (
    <Modal
      className="modal-load-logs"
      title="Loader logs"
      subtitle={gameTitle}
      onClose={onClose}
      footer={
        <div className="modal-foot-actions">
          <button
            type="button"
            onClick={() => {
              void navigator.clipboard.writeText(diagnosis.report).then(
                () => setCopied(true),
                () => setCopied(false),
              );
            }}
          >
            {copied ? "Copied" : "Copy report"}
          </button>
        </div>
      }
    >
      <p>{diagnosis.summary}</p>
      {diagnosis.excerpt ? (
        <>
          {logLabel ? (
            diagnosis.excerpt_path ? (
              <button
                type="button"
                className="linkish load-excerpt-label"
                title={diagnosis.excerpt_path}
                onClick={() => {
                  setOpenError(null);
                  void revealItemInDir(diagnosis.excerpt_path).catch((err: unknown) => {
                    console.error("Failed to open log", err);
                    setOpenError(err instanceof Error ? err.message : String(err));
                  });
                }}
              >
                {logLabel}
              </button>
            ) : (
              <span className="load-excerpt-label">{logLabel}</span>
            )
          ) : null}
          {openError ? <p className="note">{openError}</p> : null}
          <pre className="load-excerpt" aria-label={logLabel || "Loader log"}>
            {diagnosis.excerpt}
          </pre>
        </>
      ) : null}
      {diagnosis.findings.length > 0 && (
        <ul className="load-findings">
          {diagnosis.findings.map((finding, index) => (
            <li key={`${finding.title}:${index}`} className={finding.severity}>
              <strong>{finding.title}</strong>
              {finding.related_mod ? <span> · {finding.related_mod}</span> : null}
              {finding.detail ? <p className="note">{finding.detail}</p> : null}
            </li>
          ))}
        </ul>
      )}
    </Modal>
  );
}
