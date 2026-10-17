import { useLayoutEffect, useRef, useState } from "react";

import { requirementsNeedAttention } from "./RequirementsDialog";
import type { DeployResult, MissingRequirements } from "./types";

export type BannerAction = { label: string; run: () => void };

export type StatusNotice = {
  kind: "ok" | "warn";
  message: string;
  details?: string[];
  action?: BannerAction;
};

type StatusBannerProps = {
  kind: "error" | "warn" | "info";
  message: string;
  details?: string[];
  action?: BannerAction | null;
  onDismiss: () => void;
};

/** Detail lines that are not already the whole collapsed message. */
export function hiddenBannerDetails(message: string, details?: string[]): string[] {
  const lines = (details ?? []).map((line) => line.trim()).filter(Boolean);
  if (lines.length === 0) return [];
  if (lines.length === 1 && lines[0] === message.trim()) return [];
  return lines;
}

export function deployStatusNotice(
  result: DeployResult,
  onReviewRequirements: (requirements: MissingRequirements) => void,
): StatusNotice {
  const files = `${result.file_count} file${result.file_count === 1 ? "" : "s"}`;
  const mods = `${result.enabled_mods} mod${result.enabled_mods === 1 ? "" : "s"}`;
  if (result.warnings.length === 0) {
    return {
      kind: "ok",
      message: `Deployed ${files} from ${result.enabled_mods} enabled mod${result.enabled_mods === 1 ? "" : "s"}.`,
    };
  }
  const payload = reviewableRequirements(result);
  return {
    kind: "warn",
    message: `Deployed ${files} from ${mods}.`,
    details: result.warnings,
    action: payload
      ? {
          label: "Review requirements",
          run: () => onReviewRequirements(payload),
        }
      : undefined,
  };
}

function reviewableRequirements(result: DeployResult): MissingRequirements | null {
  const payload = result.missing_requirements;
  if (!payload || (result.plugin_skips?.length ?? 0) === 0) return null;
  if (!requirementsNeedAttention(payload)) return null;
  return payload;
}

export function StatusBanner({
  kind,
  message,
  details,
  action,
  onDismiss,
}: StatusBannerProps) {
  const messageRef = useRef<HTMLParagraphElement>(null);
  const [expanded, setExpanded] = useState(false);
  const [overflows, setOverflows] = useState(false);
  const hidden = hiddenBannerDetails(message, details);
  const bannerKey = `${message}\0${hidden.join("\0")}`;
  const [seenKey, setSeenKey] = useState(bannerKey);
  if (seenKey !== bannerKey) {
    setSeenKey(bannerKey);
    setExpanded(false);
    setOverflows(false);
  }

  useLayoutEffect(() => {
    const el = messageRef.current;
    if (!el || expanded) return;
    const measure = () => {
      setOverflows(el.scrollHeight > el.clientHeight + 1);
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [bannerKey, expanded]);

  const canExpand = overflows || hidden.length > 0;

  return (
    <div className={`banner ${kind}`}>
      <div className="banner-copy">
        <p
          ref={messageRef}
          className={expanded ? "banner-message expanded" : "banner-message"}
        >
          {message}
        </p>
        {expanded && hidden.length > 0 && (
          <ul className="banner-details">
            {hidden.map((line, index) => (
              <li key={`${index}:${line}`}>{line}</li>
            ))}
          </ul>
        )}
      </div>
      <span className="banner-actions">
        {action ? (
          <button className="linkish" type="button" onClick={() => action.run()}>
            {action.label}
          </button>
        ) : null}
        {canExpand ? (
          <button
            className="linkish"
            type="button"
            aria-expanded={expanded}
            onClick={() => setExpanded((open) => !open)}
          >
            {expanded ? "Show less" : "Show more"}
          </button>
        ) : null}
        <button className="linkish" type="button" onClick={onDismiss}>
          dismiss
        </button>
      </span>
    </div>
  );
}
