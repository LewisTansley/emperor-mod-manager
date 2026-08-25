import { useEffect, useRef } from "react";
import type { DownloadItem } from "./types";

export type AssistQueueEntry = {
  id: string;
  gameId: string;
  domain: string;
  modId: number;
  fileId: number;
  name: string;
  version?: string | null;
};

export type AssistQueueState = {
  batchId: string;
  label: string | null;
  entries: AssistQueueEntry[];
  head: number;
  completed: number;
  failures: { name: string; reason: string }[];
  cancelled: boolean;
  collection?: {
    slug: string;
    name: string;
    revision: number | null;
    existingModIds: string[];
    files: { modId: number; fileId: number }[];
  } | null;
  emperorShare?: {
    collectionId: string;
    name: string;
    code?: string | null;
    existingModIds: string[];
    memberIds: string[];
    files: { domain: string; modId: number; fileId: number }[];
  } | null;
};

/** Batch still has in-flight downloads after Assist sequencing finished. */
export type ActiveDownloadBatch = {
  id: string;
  label: string | null;
  collection?: AssistQueueState["collection"];
  emperorShare?: AssistQueueState["emperorShare"];
};

export function createQueueEntry(
  gameId: string,
  domain: string,
  modId: number,
  fileId: number,
  name: string,
  version?: string | null,
): AssistQueueEntry {
  return {
    id: crypto.randomUUID(),
    gameId,
    domain,
    modId,
    fileId,
    name,
    version,
  };
}

function statusLabel(status: string): string {
  switch (status) {
    case "downloading":
      return "Downloading";
    case "extracting":
      return "Installing";
    case "paused":
      return "Paused";
    case "staged":
      return "Complete";
    case "failed":
      return "Failed";
    case "cancelled":
      return "Cancelled";
    default:
      return status;
  }
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function formatSpeed(bps: number): string {
  if (bps <= 0) return "";
  return `${formatBytes(bps)}/s`;
}

function formatEta(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "";
  if (seconds < 60) return `~${Math.ceil(seconds)}s`;
  if (seconds < 3600) {
    const m = Math.floor(seconds / 60);
    const s = Math.ceil(seconds % 60);
    return `~${m}m ${s}s`;
  }
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return `~${h}h ${m}m`;
}

function progressDetail(d: DownloadItem): string {
  if (d.status === "extracting") {
    return "Installing…";
  }
  const parts: string[] = [];
  if (d.bytes_total != null && d.bytes_total > 0) {
    parts.push(`${formatBytes(d.bytes_downloaded)} / ${formatBytes(d.bytes_total)}`);
  } else if (d.bytes_downloaded > 0) {
    parts.push(formatBytes(d.bytes_downloaded));
  }
  if (d.status === "downloading" && d.speed_bps > 0) {
    parts.push(formatSpeed(d.speed_bps));
    if (d.bytes_total != null && d.bytes_total > d.bytes_downloaded) {
      const eta = (d.bytes_total - d.bytes_downloaded) / d.speed_bps;
      const etaLabel = formatEta(eta);
      if (etaLabel) parts.push(etaLabel);
    }
  }
  return parts.join(" · ");
}

function progressPercent(d: DownloadItem): number | null {
  if (d.bytes_total == null || d.bytes_total <= 0) return null;
  return Math.min(100, Math.round((d.bytes_downloaded / d.bytes_total) * 100));
}

type DownloadsWorkspaceProps = {
  downloads: DownloadItem[];
  queue: AssistQueueState | null;
  activeBatch: ActiveDownloadBatch | null;
  assistActive: boolean;
  assistHint: string | null;
  onRefresh: () => void;
  onSkip: () => void;
  onCancel: () => void;
  onCancelRemaining: () => void;
  onCancelDownload: (id: string) => void;
  onPauseDownload: (id: string) => void;
  onResumeDownload: (id: string) => void;
  onAssistHost: (el: HTMLDivElement | null) => void;
};

export function DownloadsWorkspace(props: DownloadsWorkspaceProps) {
  const {
    downloads,
    queue,
    activeBatch,
    assistActive,
    assistHint,
    onRefresh,
    onSkip,
    onCancel,
    onCancelRemaining,
    onCancelDownload,
    onPauseDownload,
    onResumeDownload,
    onAssistHost,
  } = props;

  const hostRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    onAssistHost(hostRef.current);
    return () => onAssistHost(null);
  }, [onAssistHost]);

  const current = queue && queue.head < queue.entries.length ? queue.entries[queue.head] : null;
  const pending = queue ? queue.entries.slice(queue.head + 1) : [];
  const activeDownloads = downloads.filter(
    (d) => d.status === "downloading" || d.status === "extracting" || d.status === "paused",
  );
  const recentDownloads = downloads.filter(
    (d) =>
      d.status !== "downloading" && d.status !== "extracting" && d.status !== "paused",
  );
  const showCancelRemaining =
    !queue &&
    !!activeBatch &&
    activeDownloads.some((d) => d.batch_id === activeBatch.id);

  return (
    <section className="panel downloads-workspace">
      <div className="panel-head">
        <div>
          <h1>Downloads</h1>
          {queue?.label ? (
            <p className="downloads-subtitle">
              {queue.label} — {queue.completed}/{queue.entries.length} started
            </p>
          ) : showCancelRemaining && activeBatch?.label ? (
            <p className="downloads-subtitle">
              {activeBatch.label} — finishing remaining downloads
            </p>
          ) : null}
        </div>
        <div className="panel-head-actions">
          {queue && !queue.cancelled && current ? (
            <>
              <button type="button" className="danger" onClick={onSkip}>
                Skip current
              </button>
              <button type="button" className="danger" onClick={onCancel}>
                Cancel queue
              </button>
            </>
          ) : null}
          {showCancelRemaining ? (
            <button type="button" className="danger" onClick={onCancelRemaining}>
              Cancel remaining downloads
            </button>
          ) : null}
          <button type="button" onClick={onRefresh}>
            Refresh
          </button>
        </div>
      </div>

      <div className="downloads-layout">
        <div className="downloads-active">
          <h2>Downloading & installing</h2>
          <ul className="list">
            {activeDownloads.map((d) => {
              const pct = progressPercent(d);
              const detail = progressDetail(d);
              return (
                <li key={d.id} className="downloads-active-item">
                  <div className="downloads-active-row">
                    <div className="downloads-active-meta">
                      <strong>{d.label}</strong>
                      <small>
                        {statusLabel(d.status)}
                        {pct != null ? ` · ${pct}%` : ""}
                        {detail ? ` · ${detail}` : ""}
                      </small>
                      <div
                        className={
                          pct != null
                            ? "downloads-progress"
                            : "downloads-progress downloads-progress-indeterminate"
                        }
                        role="progressbar"
                        aria-valuemin={0}
                        aria-valuemax={100}
                        aria-valuenow={pct ?? undefined}
                      >
                        <div
                          className="downloads-progress-bar"
                          style={pct != null ? { width: `${pct}%` } : undefined}
                        />
                      </div>
                    </div>
                    <div className="downloads-active-actions">
                      {d.status === "downloading" ? (
                        <button
                          type="button"
                          className="downloads-control-one"
                          onClick={() => onPauseDownload(d.id)}
                        >
                          Pause
                        </button>
                      ) : null}
                      {d.status === "paused" ? (
                        <button
                          type="button"
                          className="downloads-control-one"
                          onClick={() => onResumeDownload(d.id)}
                        >
                          Resume
                        </button>
                      ) : null}
                      <button
                        type="button"
                        className="danger downloads-cancel-one"
                        onClick={() => onCancelDownload(d.id)}
                      >
                        Cancel
                      </button>
                    </div>
                  </div>
                </li>
              );
            })}
            {activeDownloads.length === 0 && (
              <li className="empty">Nothing downloading right now.</li>
            )}
          </ul>
          {recentDownloads.length > 0 && (
            <>
              <h2 className="downloads-recent-heading">Recent</h2>
              <ul className="list downloads-recent">
                {recentDownloads.slice(0, 8).map((d) => (
                  <li key={d.id}>
                    <div>
                      <strong>{d.label}</strong>
                      <small>
                        {statusLabel(d.status)}
                        {d.error ? ` · ${d.error}` : ""}
                      </small>
                    </div>
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>

        <div className="downloads-right">
          <div className="downloads-assist-pane">
            <div className="downloads-assist-head">
              <h2>Download Assist</h2>
              {current ? <span className="pill">{current.name}</span> : null}
            </div>
            <div
              ref={hostRef}
              className={assistActive ? "downloads-assist-host active" : "downloads-assist-host"}
            >
              {!assistActive && (
                <p className="note downloads-assist-placeholder">
                  {current
                    ? "Assist panel will appear here when a free download is queued."
                    : "Install a mod to start a download, or queue collection mods below."}
                </p>
              )}
            </div>
            {assistHint ? <p className="note downloads-assist-hint">{assistHint}</p> : null}
          </div>

          <div className="downloads-queue-pane">
            <h2>Queue</h2>
            <ul className="list">
              {current && (
                <li className="downloads-queue-current">
                  <div>
                    <strong>{current.name}</strong>
                    <small>Current — opening in Assist above</small>
                  </div>
                </li>
              )}
              {pending.map((entry, i) => (
                <li key={entry.id}>
                  <div>
                    <strong>{entry.name}</strong>
                    <small>
                      Up next · #{queue!.head + i + 2}
                      {entry.version ? ` · v${entry.version}` : ""}
                    </small>
                  </div>
                </li>
              ))}
              {!current && pending.length === 0 && (
                <li className="empty">Queue is empty.</li>
              )}
            </ul>
          </div>
        </div>
      </div>
    </section>
  );
}
