import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import {
  createQueueEntry,
  DownloadsWorkspace,
  type ActiveDownloadBatch,
  type AssistQueueEntry,
  type AssistQueueState,
} from "./downloads";
import type {
  BrowseMeta,
  CollectionHit,
  CollectionModFile,
  DetectedGame,
  DownloadItem,
  ManagedGame,
  ModDetail,
  ModFileInfo,
  ModSearchHit,
  Settings,
  StagedMod,
  ThemePreference,
} from "./types";
import "./App.css";

type Tab = "setup" | "library" | "browse" | "downloads" | "settings";
type ViewMode = "list" | "grid";
type DetailTab = "info" | "files";
type GameDetailTab = "info" | "mods";

type StatusNotice = {
  kind: "ok" | "warn";
  message: string;
};

type BrowseDetail =
  | { kind: "mod"; hit: ModSearchHit; tab: DetailTab }
  | { kind: "collection"; hit: CollectionHit; tab: DetailTab };

type StatusSummary = {
  primary: string;
  secondary: string;
};

function statusSummary(
  busy: string | null,
  assistQueue: AssistQueueState | null,
): StatusSummary | null {
  if (!busy && !assistQueue) return null;

  if (assistQueue) {
    const total = assistQueue.entries.length;
    const progress = `${Math.min(assistQueue.head + 1, total)}/${total}`;
    const entry = assistQueue.entries[assistQueue.head];

    if (entry) {
      let primary = "Download Assist";
      if (busy?.startsWith("Starting")) primary = "Starting";
      else if (busy?.startsWith("Importing")) primary = "Importing";
      else if (busy?.startsWith("Waiting for Nexus")) primary = "Sign in";
      else if (busy && !busy.startsWith("Download Assist")) {
        const word = busy.split(/\s+/)[0]?.replace(/…$/, "");
        if (word) primary = word;
      }
      return { primary, secondary: `${entry.name} · ${progress}` };
    }

    return { primary: "Queue", secondary: `${progress} — opening next…` };
  }

  if (!busy) return null;

  const patterns: [RegExp, string][] = [
    [/^Downloading\s+(.+?)…?$/i, "Downloading"],
    [/^Installing\s+(?:collection\s+)?(.+?)…?$/i, "Installing"],
    [/^Loading\s+(?:collection\s+)?(.+?)…?$/i, "Loading"],
    [/^Download Assist\s*[—\-]\s*(.+)$/i, "Download Assist"],
  ];
  for (const [re, primary] of patterns) {
    const match = busy.match(re);
    if (match?.[1]) return { primary, secondary: match[1] };
  }

  const space = busy.indexOf(" ");
  if (space > 0) {
    return {
      primary: busy.slice(0, space).replace(/…$/, ""),
      secondary: busy.slice(space + 1).replace(/…$/, ""),
    };
  }
  return { primary: busy, secondary: "" };
}

const MOD_SORTS = [
  { value: "endorsements", label: "Most popular" },
  { value: "downloads", label: "Most downloaded" },
  { value: "createdAt", label: "Newest" },
  { value: "updatedAt", label: "Recently updated" },
  { value: "relevance", label: "Best match" },
  { value: "trending", label: "Trending" },
] as const;

const COLLECTION_SORTS = [
  { value: "endorsements", label: "Most popular" },
  { value: "downloads", label: "Most downloaded" },
  { value: "createdAt", label: "Newest" },
  { value: "updatedAt", label: "Recently updated" },
  { value: "relevance", label: "Best match" },
  { value: "rating", label: "Highest rated" },
] as const;

function resolveTheme(pref: ThemePreference): "light" | "dark" {
  if (pref === "light" || pref === "dark") return pref;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function applyTheme(pref: ThemePreference) {
  document.documentElement.dataset.theme = resolveTheme(pref);
}

function mediaSrc(localPath?: string | null, remoteUrl?: string | null): string | null {
  if (remoteUrl) return remoteUrl;
  if (localPath) {
    try {
      return convertFileSrc(localPath);
    } catch {
      return null;
    }
  }
  return null;
}

function shortLauncher(launcher: string): string {
  const l = launcher.toLowerCase();
  if (l.includes("steam")) return "Steam";
  if (l.includes("heroic")) return "Heroic";
  if (l.includes("lutris")) return "Lutris";
  if (l.includes("bottles")) return "Bottles";
  return launcher.split("(")[0]?.trim() || launcher;
}

function PlaceholderIcon() {
  return (
    <div className="media-placeholder" aria-hidden>
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5">
        <rect x="3" y="6" width="18" height="12" rx="2" />
        <path d="M8 12h.01M12 10v4M16 12h.01" strokeLinecap="round" />
      </svg>
    </div>
  );
}

function stripHtml(html: string): string {
  return html
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<\/p>/gi, "\n\n")
    .replace(/<[^>]+>/g, "")
    .replace(/&nbsp;/g, " ")
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .trim();
}

function formatTimestamp(ts: number | null | undefined): string | null {
  if (ts == null || ts <= 0) return null;
  try {
    return new Date(ts * 1000).toLocaleDateString();
  } catch {
    return null;
  }
}

function groupCollectionMods(files: CollectionModFile[]) {
  const map = new Map<
    number,
    { mod_id: number; mod_name: string; domain_name: string; files: CollectionModFile[] }
  >();
  for (const f of files) {
    const existing = map.get(f.mod_id);
    if (existing) {
      existing.files.push(f);
    } else {
      map.set(f.mod_id, {
        mod_id: f.mod_id,
        mod_name: f.mod_name,
        domain_name: f.domain_name,
        files: [f],
      });
    }
  }
  return [...map.values()];
}

function MediaCard(props: {
  title: string;
  imageSrc: string | null;
  badge?: string | null;
  overlay?: string | null;
  tags?: string[];
  actions?: ReactNode;
  coverLabel?: string | null;
  hoverLabel?: string | null;
  unsupported?: boolean;
  onClick?: () => void;
}) {
  const {
    title,
    imageSrc,
    badge,
    overlay,
    tags,
    actions,
    coverLabel,
    hoverLabel,
    unsupported,
    onClick,
  } = props;
  const classes = [
    "media-card",
    onClick ? "media-card-clickable" : "",
    unsupported ? "media-card-unsupported" : "",
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <article
      className={classes}
      onClick={onClick}
      onKeyDown={
        onClick
          ? (e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onClick();
              }
            }
          : undefined
      }
      role={onClick ? "button" : undefined}
      tabIndex={onClick ? 0 : undefined}
    >
      <div className="media-cover">
        {badge ? <span className="media-badge">{badge}</span> : null}
        {imageSrc ? (
          <img src={imageSrc} alt="" loading="lazy" onError={(e) => {
            (e.currentTarget as HTMLImageElement).style.display = "none";
          }} />
        ) : (
          <PlaceholderIcon />
        )}
        {overlay ? <span className="media-overlay">{overlay}</span> : null}
        {coverLabel ? (
          <span className="media-cover-label">{coverLabel}</span>
        ) : null}
        {hoverLabel ? (
          <span className="media-cover-label media-cover-hover">{hoverLabel}</span>
        ) : null}
      </div>
      <div className="media-card-title">{title}</div>
      {tags && tags.length > 0 && (
        <div className="media-card-tags">
          {tags.map((t) => (
            <span key={t} className="pill">
              {t}
            </span>
          ))}
        </div>
      )}
      {actions ? (
        <div className="actions" onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
          {actions}
        </div>
      ) : null}
    </article>
  );
}

function App() {
  const [tab, setTab] = useState<Tab>("setup");
  const [settings, setSettings] = useState<Settings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<StatusNotice | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [apiKeyInput, setApiKeyInput] = useState("");
  const [detected, setDetected] = useState<DetectedGame[]>([]);
  const [managed, setManaged] = useState<ManagedGame[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [mods, setMods] = useState<StagedMod[]>([]);
  const [downloads, setDownloads] = useState<DownloadItem[]>([]);
  const [searchQuery, setSearchQuery] = useState("");
  const [modHits, setModHits] = useState<ModSearchHit[]>([]);
  const [collectionHits, setCollectionHits] = useState<CollectionHit[]>([]);
  const [browseMode, setBrowseMode] = useState<"mods" | "collections">("mods");
  const [libraryView, setLibraryView] = useState<ViewMode>("grid");
  const [browseView, setBrowseView] = useState<ViewMode>("grid");
  const [browseSort, setBrowseSort] = useState("endorsements");
  const [browseCategory, setBrowseCategory] = useState("");
  const [browseVersion, setBrowseVersion] = useState("");
  const [browseTags, setBrowseTags] = useState<string[]>([]);
  const [tagQuery, setTagQuery] = useState("");
  const [tagsOpen, setTagsOpen] = useState(false);
  const [browseMeta, setBrowseMeta] = useState<BrowseMeta | null>(null);
  const [browseDetail, setBrowseDetail] = useState<BrowseDetail | null>(null);
  const [libraryDetail, setLibraryDetail] = useState<{
    game: ManagedGame;
    tab: GameDetailTab;
  } | null>(null);
  const [modDetail, setModDetail] = useState<ModDetail | null>(null);
  const [modFiles, setModFiles] = useState<ModFileInfo[]>([]);
  const [collectionModFiles, setCollectionModFiles] = useState<CollectionModFile[]>([]);
  const [includeOptional, setIncludeOptional] = useState(false);
  const [assistQueue, setAssistQueue] = useState<AssistQueueState | null>(null);
  const [activeBatch, setActiveBatch] = useState<ActiveDownloadBatch | null>(null);
  const [assistHint, setAssistHint] = useState<string | null>(null);
  const [assistActive, setAssistActive] = useState(false);
  const assistQueueRef = useRef<AssistQueueState | null>(null);
  const activeBatchRef = useRef<ActiveDownloadBatch | null>(null);
  activeBatchRef.current = activeBatch;
  const assistHostRef = useRef<HTMLDivElement | null>(null);
  const tabRef = useRef(tab);
  tabRef.current = tab;
  const assistSyncGenRef = useRef(0);
  const downloadStartedAtRef = useRef<number>(0);
  const tagPickerRef = useRef<HTMLDivElement | null>(null);
  const tagSearchRef = useRef<HTMLInputElement | null>(null);

  const activeGame = useMemo(
    () => managed.find((g) => g.id === activeId) ?? managed[0] ?? null,
    [managed, activeId],
  );
  const activeGameRef = useRef(activeGame);
  activeGameRef.current = activeGame;

  const themePref: ThemePreference = settings?.theme ?? "system";

  const refreshSettings = useCallback(async () => {
    const s = await api.getSettings();
    setSettings(s);
    setActiveId(s.last_active_game_id);
    applyTheme(s.theme ?? "system");
    return s;
  }, []);

  const refreshManaged = useCallback(async () => {
    const list = await api.listManaged();
    setManaged(list);
    return list;
  }, []);

  const refreshMods = useCallback(async (gameId: string) => {
    const list = await api.listMods(gameId);
    setMods(list);
  }, []);

  const refreshDownloads = useCallback(async () => {
    setDownloads(await api.listDownloads());
  }, []);

  useEffect(() => {
    (async () => {
      try {
        const s = await refreshSettings();
        await refreshManaged();
        if (s.has_api_key) {
          try {
            await api.validateUser();
            await refreshSettings();
          } catch {
            /* key may be stale */
          }
          setTab("library");
        }
      } catch (e) {
        setError(String(e));
      }
    })();
  }, [refreshManaged, refreshSettings]);

  useEffect(() => {
    if (themePref !== "system") {
      applyTheme(themePref);
      return;
    }
    applyTheme("system");
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => applyTheme("system");
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [themePref]);

  useEffect(() => {
    if (activeGame) {
      refreshMods(activeGame.id).catch((e) => setError(String(e)));
      setBrowseDetail(null);
      setModDetail(null);
      setModFiles([]);
      setCollectionModFiles([]);
    }
  }, [activeGame, refreshMods]);

  useEffect(() => {
    setLibraryDetail((prev) => {
      if (!prev) return prev;
      const still = managed.find((g) => g.id === prev.game.id);
      if (!still) return null;
      if (still === prev.game) return prev;
      return { ...prev, game: still };
    });
  }, [managed]);

  useEffect(() => {
    if (!activeGame || tab !== "browse") return;
    let cancelled = false;
    (async () => {
      try {
        const meta = await api.browseMeta(activeGame.nexus_domain);
        if (!cancelled) setBrowseMeta(meta);
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [activeGame, tab]);

  useEffect(() => {
    if (tab !== "browse") setTagsOpen(false);
  }, [tab]);

  useEffect(() => {
    if (!tagsOpen) return;
    const onPointerDown = (e: MouseEvent) => {
      if (tagPickerRef.current && !tagPickerRef.current.contains(e.target as Node)) {
        setTagsOpen(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setTagsOpen(false);
    };
    document.addEventListener("mousedown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    const id = window.setTimeout(() => tagSearchRef.current?.focus(), 0);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
      window.clearTimeout(id);
    };
  }, [tagsOpen]);

  const isPremium = settings?.user?.is_premium ?? false;

  const syncAssistHost = useCallback(async () => {
    const gen = assistSyncGenRef.current;
    const host = assistHostRef.current;
    if (!host || tabRef.current !== "downloads") return;
    const rect = host.getBoundingClientRect();
    if (rect.width < 8 || rect.height < 8) {
      api.setAssistVisible(false).catch(() => {});
      return;
    }
    const cs = getComputedStyle(host);
    const bl = parseFloat(cs.borderLeftWidth) || 0;
    const bt = parseFloat(cs.borderTopWidth) || 0;
    const br = parseFloat(cs.borderRightWidth) || 0;
    const bb = parseFloat(cs.borderBottomWidth) || 0;

    let left = rect.left + bl;
    let top = rect.top + bt;
    let right = rect.right - br;
    let bottom = rect.bottom - bb;

    const clipsOverflow = (value: string) =>
      value === "hidden" || value === "clip" || value === "auto" || value === "scroll";

    let ancestor: HTMLElement | null = host.parentElement;
    while (ancestor) {
      const style = getComputedStyle(ancestor);
      if (
        clipsOverflow(style.overflow) ||
        clipsOverflow(style.overflowX) ||
        clipsOverflow(style.overflowY)
      ) {
        const a = ancestor.getBoundingClientRect();
        left = Math.max(left, a.left);
        top = Math.max(top, a.top);
        right = Math.min(right, a.right);
        bottom = Math.min(bottom, a.bottom);
      }
      ancestor = ancestor.parentElement;
    }

    left = Math.max(left, 0);
    top = Math.max(top, 0);
    right = Math.min(right, window.innerWidth);
    bottom = Math.min(bottom, window.innerHeight);

    const width = Math.max(0, Math.floor(right - left));
    const height = Math.max(0, Math.floor(bottom - top));
    if (width < 8 || height < 8) {
      api.setAssistVisible(false).catch(() => {});
      return;
    }

    try {
      await api.setAssistBounds({
        x: Math.round(left),
        y: Math.round(top),
        width,
        height,
      });
      if (gen !== assistSyncGenRef.current || tabRef.current !== "downloads") {
        api.setAssistVisible(false).catch(() => {});
        return;
      }
      await api.setAssistVisible(true);
    } catch {
      /* host may not be ready yet */
    }
  }, []);

  useEffect(() => {
    if (tab !== "downloads") {
      assistSyncGenRef.current += 1;
      api.setAssistVisible(false).catch(() => {});
      return;
    }
    api.setAssistVisible(true).catch(() => {});
    const syncSoon = () => {
      requestAnimationFrame(() => {
        if (tabRef.current !== "downloads") return;
        void syncAssistHost();
        requestAnimationFrame(() => {
          if (tabRef.current !== "downloads") return;
          void syncAssistHost();
        });
      });
    };
    syncSoon();
    const onResize = () => syncSoon();
    window.addEventListener("resize", onResize);
    window.addEventListener("scroll", onResize, true);
    window.visualViewport?.addEventListener("resize", onResize);
    window.visualViewport?.addEventListener("scroll", onResize);
    const observer = assistHostRef.current
      ? new ResizeObserver(() => syncSoon())
      : null;
    if (assistHostRef.current && observer) {
      observer.observe(assistHostRef.current);
    }
    return () => {
      assistSyncGenRef.current += 1;
      window.removeEventListener("resize", onResize);
      window.removeEventListener("scroll", onResize, true);
      window.visualViewport?.removeEventListener("resize", onResize);
      window.visualViewport?.removeEventListener("scroll", onResize);
      observer?.disconnect();
    };
  }, [tab, syncAssistHost, assistActive, assistQueue?.head]);

  const setAssistQueueState = useCallback((next: AssistQueueState | null) => {
    assistQueueRef.current = next;
    setAssistQueue(next);
  }, []);

  const finishAssistQueue = useCallback(async () => {
    const q = assistQueueRef.current;
    setAssistQueueState(null);
    setAssistActive(false);
    setAssistHint(null);
    setBusy(null);
    const list = await api.listDownloads();
    setDownloads(list);
    if (q && !q.cancelled) {
      const stillActive = list.some(
        (d) =>
          d.batch_id === q.batchId &&
          (d.status === "downloading" || d.status === "extracting"),
      );
      if (stillActive) {
        setActiveBatch({ id: q.batchId, label: q.label });
      } else {
        setActiveBatch((prev) => (prev?.id === q.batchId ? null : prev));
      }
    } else if (q?.cancelled) {
      setActiveBatch((prev) => (prev?.id === q.batchId ? null : prev));
    }
    const game = activeGameRef.current;
    if (game) await refreshMods(game.id);
    if (!q) return;
    if (q.cancelled) {
      setError(`Download queue cancelled after ${q.completed}/${q.entries.length} mods.`);
      return;
    }
    if (q.failures.length > 0) {
      const preview = q.failures
        .slice(0, 3)
        .map((f) => `${f.name}: ${f.reason}`)
        .join("; ");
      setError(
        `${q.label ?? "Queue"}: ${q.completed} started, ${q.failures.length} failed. ${preview}${
          q.failures.length > 3 ? "…" : ""
        }`,
      );
    } else if (q.completed > 0) {
      setError(null);
    }
  }, [refreshMods, setAssistQueueState]);

  const openAssistAtHead = useCallback(
    async (q: AssistQueueState) => {
      const entry = q.entries[q.head];
      if (!entry) {
        await finishAssistQueue();
        return;
      }
      setAssistActive(true);
      await new Promise<void>((resolve) => {
        requestAnimationFrame(() => {
          requestAnimationFrame(() => resolve());
        });
      });
      await syncAssistHost();
      setBusy(`Download Assist — ${entry.name}`);
      await api.openDownloadAssist({
        gameId: entry.gameId,
        domain: entry.domain,
        modId: entry.modId,
        fileId: entry.fileId,
        name: entry.name,
        batchId: q.batchId,
      });
    },
    [finishAssistQueue, syncAssistHost],
  );

  const advanceAssistAfterStart = useCallback(async () => {
    const q = assistQueueRef.current;
    if (!q) {
      await refreshDownloads();
      setAssistActive(false);
      setBusy(null);
      return;
    }
    if (q.cancelled) {
      await finishAssistQueue();
      return;
    }
    q.completed += 1;
    q.head += 1;
    setAssistQueueState({ ...q });
    await refreshDownloads();
    if (q.head >= q.entries.length) {
      await finishAssistQueue();
      return;
    }
    try {
      await openAssistAtHead(q);
    } catch (e) {
      q.failures.push({
        name: q.entries[q.head]?.name ?? "Unknown",
        reason: String(e),
      });
      await finishAssistQueue();
    }
  }, [finishAssistQueue, openAssistAtHead, refreshDownloads, setAssistQueueState]);

  const advanceAssistAfterFailure = useCallback(
    async (reason: string) => {
      const q = assistQueueRef.current;
      if (!q) {
        setAssistActive(false);
        setBusy(null);
        return;
      }
      if (q.cancelled) {
        await finishAssistQueue();
        return;
      }
      const entry = q.entries[q.head];
      q.failures.push({ name: entry?.name ?? "Unknown", reason });
      q.head += 1;
      setAssistQueueState({ ...q });
      if (q.head >= q.entries.length) {
        await finishAssistQueue();
        return;
      }
      try {
        await openAssistAtHead(q);
      } catch (e) {
        q.failures.push({
          name: q.entries[q.head]?.name ?? "Unknown",
          reason: String(e),
        });
        await finishAssistQueue();
      }
    },
    [finishAssistQueue, openAssistAtHead, setAssistQueueState],
  );

  const skipAssistCurrent = useCallback(async () => {
    const q = assistQueueRef.current;
    if (!q || q.cancelled) return;
    const entry = q.entries[q.head];
    if (entry) {
      q.failures.push({ name: entry.name, reason: "Skipped by user" });
    }
    q.head += 1;
    setAssistQueueState({ ...q });
    try {
      await api.closeDownloadAssist();
    } catch {
      /* ignore */
    }
    if (q.head >= q.entries.length) {
      await finishAssistQueue();
      return;
    }
    try {
      await openAssistAtHead(q);
    } catch (e) {
      setError(String(e));
    }
  }, [finishAssistQueue, openAssistAtHead, setAssistQueueState]);

  const cancelAssistQueue = useCallback(async () => {
    const q = assistQueueRef.current;
    if (!q) return;
    q.cancelled = true;
    setAssistQueueState({ ...q });
    try {
      await api.closeDownloadAssist();
    } catch {
      /* ignore */
    }
    try {
      await api.cancelDownloadBatch(q.batchId);
    } catch {
      /* ignore */
    }
    await finishAssistQueue();
  }, [finishAssistQueue, setAssistQueueState]);

  const cancelRemainingBatch = useCallback(async () => {
    const batch = activeBatchRef.current;
    if (!batch) return;
    try {
      await api.cancelDownloadBatch(batch.id);
    } catch (e) {
      setError(String(e));
    }
    setActiveBatch(null);
    await refreshDownloads();
  }, [refreshDownloads]);

  const cancelOneDownload = useCallback(
    async (id: string) => {
      try {
        await api.cancelDownload(id);
      } catch (e) {
        setError(String(e));
      }
      await refreshDownloads();
    },
    [refreshDownloads],
  );

  const handleAssistHost = useCallback(
    (el: HTMLDivElement | null) => {
      assistHostRef.current = el;
      if (el) {
        syncAssistHost();
      }
    },
    [syncAssistHost],
  );

  const enqueueAssistEntries = useCallback(
    async (entries: AssistQueueEntry[], label: string | null = null) => {
      if (entries.length === 0) {
        throw new Error("Nothing to queue.");
      }
      if (assistQueueRef.current && !assistQueueRef.current.cancelled) {
        throw new Error("A download queue is already in progress.");
      }
      const q: AssistQueueState = {
        batchId: crypto.randomUUID(),
        label,
        entries,
        head: 0,
        completed: 0,
        failures: [],
        cancelled: false,
      };
      setActiveBatch(null);
      setAssistQueueState(q);
      setTab("downloads");
      setError(null);
      await openAssistAtHead(q);
    },
    [openAssistAtHead, setAssistQueueState],
  );

  useEffect(() => {
    const unlistenNxm = listen<string>("nxm-url", async (event) => {
      const q = assistQueueRef.current;
      setBusy(
        q
          ? `Starting queued download ${q.head + 1}/${q.entries.length}…`
          : "Starting download…",
      );
      setError(null);
      try {
        await api.handleNxm(event.payload);
        downloadStartedAtRef.current = Date.now();
        await refreshDownloads();
      } catch (e) {
        const msg = String(e);
        if (msg.includes("DUPLICATE_NXM")) {
          await refreshDownloads();
          return;
        }
        await refreshDownloads();
        if (assistQueueRef.current) {
          try {
            await api.closeDownloadAssist();
          } catch {
            /* ignore */
          }
          await advanceAssistAfterFailure(msg);
        } else {
          setError(msg);
          setBusy(null);
        }
      }
    });
    const unlistenAssist = listen<string>("assist-download-finished", async (event) => {
      const q = assistQueueRef.current;
      setBusy(
        q
          ? `Importing queued mod ${q.head + 1}/${q.entries.length}…`
          : "Importing Slow Download…",
      );
      setError(null);
      try {
        await api.importAssistDownload(event.payload);
        downloadStartedAtRef.current = Date.now();
        await refreshDownloads();
        await advanceAssistAfterStart();
      } catch (e) {
        await refreshDownloads();
        if (assistQueueRef.current) {
          try {
            await api.closeDownloadAssist();
          } catch {
            /* ignore */
          }
          await advanceAssistAfterFailure(String(e));
        } else {
          setError(String(e));
          setBusy(null);
        }
      }
    });
    const unlistenOpened = listen<{ autoclick: boolean }>("assist-opened", (event) => {
      setAssistActive(true);
      const q = assistQueueRef.current;
      const entry = q?.entries[q.head];
      setAssistHint(
        event.payload.autoclick
          ? "Autoclick enabled — opening dialog then Slow download when ready."
          : "Click Mod Manager Download or Slow download in the panel above.",
      );
      if (q && entry) {
        setBusy(
          event.payload.autoclick
            ? `Assist ${q.head + 1}/${q.entries.length} — ${entry.name} (autoclick)…`
            : `Assist ${q.head + 1}/${q.entries.length} — ${entry.name}…`,
        );
        return;
      }
      setBusy(
        event.payload.autoclick
          ? "Download Assist open — autoclick will press download when ready…"
          : "Download Assist open — complete the download in the panel above…",
      );
    });
    const unlistenNeedsLogin = listen("assist-needs-login", () => {
      setAssistHint(
        "Sign in to Nexus in Download Assist once (use Stay signed in). Session is saved separately from your API key.",
      );
      setBusy("Waiting for Nexus sign-in in Download Assist…");
    });
    const unlistenClosed = listen("assist-closed", async () => {
      if (Date.now() - downloadStartedAtRef.current < 2500) {
        return;
      }
      if (!assistQueueRef.current || assistQueueRef.current.cancelled) {
        setAssistActive(false);
        setBusy(null);
        return;
      }
      await advanceAssistAfterFailure("Assist closed unexpectedly");
    });
    const unlistenDlFinished = listen<{ game_id?: string; id?: string }>(
      "download-finished",
      async (event) => {
        const list = await api.listDownloads();
        setDownloads(list);
        const batch = activeBatchRef.current;
        if (batch) {
          const still = list.some(
            (d) =>
              d.batch_id === batch.id &&
              (d.status === "downloading" || d.status === "extracting"),
          );
          if (!still) setActiveBatch(null);
        }
        const gameId = event.payload.game_id ?? activeGameRef.current?.id;
        if (gameId) {
          try {
            await refreshMods(gameId);
          } catch {
            /* ignore */
          }
        }
      },
    );
    const unlistenDlFailed = listen<{ label?: string; error?: string }>(
      "download-failed",
      async (event) => {
        const list = await api.listDownloads();
        setDownloads(list);
        const batch = activeBatchRef.current;
        if (batch) {
          const still = list.some(
            (d) =>
              d.batch_id === batch.id &&
              (d.status === "downloading" || d.status === "extracting"),
          );
          if (!still) setActiveBatch(null);
        }
        const label = event.payload.label ?? "Download";
        const err = event.payload.error ?? "unknown error";
        setError(`${label}: ${err}`);
      },
    );
    const unlistenDlCancelled = listen<{ id?: string; label?: string }>(
      "download-cancelled",
      async () => {
        const list = await api.listDownloads();
        setDownloads(list);
        const batch = activeBatchRef.current;
        if (batch) {
          const still = list.some(
            (d) =>
              d.batch_id === batch.id &&
              (d.status === "downloading" || d.status === "extracting"),
          );
          if (!still) setActiveBatch(null);
        }
      },
    );
    const unlistenProgress = listen<{
      id: string;
      bytes_downloaded: number;
      bytes_total: number | null;
      speed_bps: number;
      status: string;
    }>("download-progress", (event) => {
      const p = event.payload;
      setDownloads((prev) => {
        const idx = prev.findIndex((d) => d.id === p.id);
        if (idx < 0) return prev;
        const next = [...prev];
        next[idx] = {
          ...next[idx],
          bytes_downloaded: p.bytes_downloaded,
          bytes_total: p.bytes_total,
          speed_bps: p.speed_bps,
          status: p.status || next[idx].status,
        };
        return next;
      });
    });
    const unlistenStarted = listen("assist-download-started", async () => {
      downloadStartedAtRef.current = Date.now();
      setAssistActive(false);
      setAssistHint(null);
      await refreshDownloads();
      const q = assistQueueRef.current;
      if (q && !q.cancelled && q.head + 1 < q.entries.length) {
        const next = q.entries[q.head + 1];
        setBusy(
          `Queue ${q.head + 2}/${q.entries.length} — opening next${next ? ` (${next.name})` : ""}…`,
        );
      } else if (!q) {
        setBusy(null);
      }
      await advanceAssistAfterStart();
    });
    return () => {
      unlistenNxm.then((f) => f());
      unlistenAssist.then((f) => f());
      unlistenOpened.then((f) => f());
      unlistenNeedsLogin.then((f) => f());
      unlistenClosed.then((f) => f());
      unlistenDlFinished.then((f) => f());
      unlistenDlFailed.then((f) => f());
      unlistenDlCancelled.then((f) => f());
      unlistenProgress.then((f) => f());
      unlistenStarted.then((f) => f());
    };
  }, [advanceAssistAfterFailure, advanceAssistAfterStart, refreshDownloads, refreshMods]);

  async function withBusy<T>(label: string, fn: () => Promise<T>): Promise<T | undefined> {
    setBusy(label);
    setError(null);
    setNotice(null);
    try {
      return await fn();
    } catch (e) {
      setError(String(e));
      return undefined;
    } finally {
      setBusy(null);
    }
  }

  async function saveApiKey() {
    await withBusy("Validating API key…", async () => {
      await api.setApiKey(apiKeyInput);
      setApiKeyInput("");
      await refreshSettings();
      setTab("library");
    });
  }

  async function scan() {
    await withBusy("Scanning installed games…", async () => {
      setDetected(await api.scanGames());
    });
  }

  async function manage(game: DetectedGame) {
    if (!game.supported || !game.plugin_id || !game.nexus_domain || !game.install_path) {
      setError("This game is detected but not supported yet.");
      return;
    }
    await withBusy(`Managing ${game.title}…`, async () => {
      await api.manageGame({
        id: game.id,
        title: game.title,
        nexusDomain: game.nexus_domain!,
        installPath: game.install_path!,
        launcher: game.launcher,
        pluginId: game.plugin_id!,
        coverPath: game.cover_path,
      });
      const list = await refreshManaged();
      const managedGame = list.find((g) => g.id === game.id);
      if (managedGame) {
        await openGame(managedGame, "mods");
      } else {
        setActiveId(game.id);
      }
    });
  }

  const browseOpts = useMemo(
    () => ({
      sort: browseSort,
      category: browseCategory || null,
      tags: browseTags,
      gameVersion: browseVersion || null,
    }),
    [browseSort, browseCategory, browseTags, browseVersion],
  );

  const orderedTags = useMemo(() => {
    const all = browseMeta?.tags ?? [];
    const q = tagQuery.trim().toLowerCase();
    const filtered = q ? all.filter((t) => t.toLowerCase().includes(q)) : all;
    return [...filtered].sort((a, b) => {
      const ac = browseTags.includes(a) ? 0 : 1;
      const bc = browseTags.includes(b) ? 0 : 1;
      if (ac !== bc) return ac - bc;
      return a.localeCompare(b, undefined, { sensitivity: "base" });
    });
  }, [browseMeta?.tags, browseTags, tagQuery]);

  async function runSearch(
    mode: "mods" | "collections" = browseMode,
    query = searchQuery,
    opts = browseOpts,
  ) {
    if (!activeGame) {
      setError("Manage a game first.");
      return;
    }
    await withBusy("Searching Nexus…", async () => {
      if (mode === "mods") {
        setModHits(await api.searchMods(activeGame.nexus_domain, query, opts));
      } else {
        setCollectionHits(
          await api.searchCollections(activeGame.nexus_domain, query, opts),
        );
      }
    });
  }

  async function search() {
    await runSearch();
  }

  function toggleBrowseTag(tag: string) {
    const tags = browseTags.includes(tag)
      ? browseTags.filter((t) => t !== tag)
      : [...browseTags, tag];
    setBrowseTags(tags);
    runSearch(browseMode, searchQuery, { ...browseOpts, tags });
  }

  function clearBrowseFilters() {
    setBrowseCategory("");
    setBrowseVersion("");
    setBrowseTags([]);
    setTagQuery("");
    setTagsOpen(false);
    runSearch(browseMode, searchQuery, {
      ...browseOpts,
      category: null,
      gameVersion: null,
      tags: [],
    });
  }

  async function openMod(hit: ModSearchHit, detailTab: DetailTab = "info") {
    if (!activeGame) return;
    setBrowseDetail({ kind: "mod", hit, tab: detailTab });
    setModDetail(null);
    setCollectionModFiles([]);
    if (detailTab === "files") {
      setModFiles([]);
      await withBusy("Loading files…", async () => {
        setModFiles(await api.modFiles(hit.domain_name || activeGame.nexus_domain, hit.mod_id));
      });
    } else {
      await withBusy("Loading mod…", async () => {
        setModDetail(await api.getMod(hit.domain_name || activeGame.nexus_domain, hit.mod_id));
      });
    }
  }

  async function openCollection(hit: CollectionHit, detailTab: DetailTab = "info") {
    if (!activeGame) return;
    setBrowseDetail({ kind: "collection", hit, tab: detailTab });
    setModDetail(null);
    setModFiles([]);
    if (detailTab === "files") {
      setCollectionModFiles([]);
      await withBusy("Loading collection mods…", async () => {
        setCollectionModFiles(
          await api.collectionFiles({
            slug: hit.slug,
            revision: hit.revision_number,
          }),
        );
      });
    } else {
      setCollectionModFiles([]);
    }
  }

  async function setDetailTab(detailTab: DetailTab) {
    if (!browseDetail || !activeGame) return;
    if (browseDetail.tab === detailTab) return;
    const next = { ...browseDetail, tab: detailTab };
    setBrowseDetail(next);
    if (next.kind === "mod") {
      if (detailTab === "files" && modFiles.length === 0) {
        await withBusy("Loading files…", async () => {
          setModFiles(
            await api.modFiles(
              next.hit.domain_name || activeGame.nexus_domain,
              next.hit.mod_id,
            ),
          );
        });
      } else if (detailTab === "info" && !modDetail) {
        await withBusy("Loading mod…", async () => {
          setModDetail(
            await api.getMod(
              next.hit.domain_name || activeGame.nexus_domain,
              next.hit.mod_id,
            ),
          );
        });
      }
    } else if (detailTab === "files" && collectionModFiles.length === 0) {
      await withBusy("Loading collection mods…", async () => {
        setCollectionModFiles(
          await api.collectionFiles({
            slug: next.hit.slug,
            revision: next.hit.revision_number,
          }),
        );
      });
    }
  }

  function closeBrowseDetail() {
    setBrowseDetail(null);
    setModDetail(null);
    setModFiles([]);
    setCollectionModFiles([]);
  }

  async function openGame(game: ManagedGame, detailTab: GameDetailTab = "info") {
    await api.setActiveGame(game.id);
    setActiveId(game.id);
    setLibraryDetail({ game, tab: detailTab });
    setTab("library");
  }

  function setGameDetailTab(detailTab: GameDetailTab) {
    setLibraryDetail((prev) => (prev ? { ...prev, tab: detailTab } : prev));
  }

  function closeLibraryDetail() {
    setLibraryDetail(null);
  }

  async function installFile(file: ModFileInfo) {
    if (!activeGame || browseDetail?.kind !== "mod") return;
    const selectedMod = browseDetail.hit;
    const domain = selectedMod.domain_name || activeGame.nexus_domain;
    const name = selectedMod.name;
    setTab("downloads");
    if (isPremium) {
      setBusy(`Downloading ${file.name}…`);
      setError(null);
      try {
        await api.downloadMod({
          gameId: activeGame.id,
          domain,
          modId: selectedMod.mod_id,
          fileId: file.file_id,
          name,
          version: file.version,
        });
        await refreshMods(activeGame.id);
        await refreshDownloads();
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(null);
      }
    } else {
      try {
        await enqueueAssistEntries(
          [
            createQueueEntry(
              activeGame.id,
              domain,
              selectedMod.mod_id,
              file.file_id,
              name,
              file.version,
            ),
          ],
          null,
        );
      } catch (e) {
        setError(String(e));
        setBusy(null);
      }
    }
  }

  async function installCollection(c: CollectionHit) {
    if (!activeGame) return;
    setTab("downloads");
    if (isPremium) {
      setBusy(`Installing collection ${c.name}…`);
      setError(null);
      try {
        await api.installCollection({
          gameId: activeGame.id,
          slug: c.slug,
          revision: c.revision_number,
          includeOptional,
        });
        await refreshMods(activeGame.id);
        await refreshDownloads();
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(null);
      }
      return;
    }

    setBusy(`Loading collection ${c.name}…`);
    setError(null);
    try {
      const allFiles = await api.collectionFiles({
        slug: c.slug,
        revision: c.revision_number,
      });
      const filesToInstall = allFiles.filter((f) => includeOptional || !f.optional);
      if (filesToInstall.length === 0) {
        throw new Error("No mods to install in this collection (try including optional mods).");
      }
      const entries = filesToInstall.map((f) =>
        createQueueEntry(
          activeGame.id,
          f.domain_name,
          f.mod_id,
          f.file_id,
          f.mod_name,
          f.version,
        ),
      );
      await enqueueAssistEntries(entries, c.name);
    } catch (e) {
      setAssistQueueState(null);
      setBusy(null);
      setError(String(e));
    }
  }

  async function importArchive() {
    if (!activeGame) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({
      multiple: false,
      filters: [{ name: "Archives", extensions: ["zip", "7z", "rar"] }],
    });
    if (!selected || Array.isArray(selected)) return;
    await withBusy("Importing archive…", async () => {
      await api.importModArchive({ gameId: activeGame.id, path: selected });
      await refreshMods(activeGame.id);
    });
  }

  async function toggleMod(m: StagedMod) {
    if (!activeGame) return;
    await api.setModEnabled(activeGame.id, m.id, !m.enabled);
    await refreshMods(activeGame.id);
  }

  async function moveMod(index: number, dir: -1 | 1) {
    if (!activeGame) return;
    const next = [...mods];
    const j = index + dir;
    if (j < 0 || j >= next.length) return;
    [next[index], next[j]] = [next[j], next[index]];
    await api.setLoadOrder(
      activeGame.id,
      next.map((m) => m.id),
    );
    setMods(next);
  }

  async function deploy() {
    if (!activeGame) return;
    await withBusy("Deploying mods…", async () => {
      const result = await api.deployMods(activeGame.id);
      if (result.warnings.length > 0) {
        const preview = result.warnings.slice(0, 3).join(" · ");
        setNotice({
          kind: "warn",
          message: `Deployed ${result.file_count} file${result.file_count === 1 ? "" : "s"} from ${result.enabled_mods} mod${result.enabled_mods === 1 ? "" : "s"}. ${preview}${result.warnings.length > 3 ? "…" : ""}`,
        });
      } else {
        setNotice({
          kind: "ok",
          message: `Deployed ${result.file_count} file${result.file_count === 1 ? "" : "s"} from ${result.enabled_mods} enabled mod${result.enabled_mods === 1 ? "" : "s"}.`,
        });
      }
    });
  }

  async function purge() {
    if (!activeGame) return;
    await withBusy("Purging deployed files…", async () => {
      await api.purgeMods(activeGame.id);
      setNotice({ kind: "ok", message: "Purged deployed files." });
    });
  }

  async function removeAllMods() {
    if (!activeGame || mods.length === 0) return;
    await withBusy("Removing all mods…", async () => {
      await api.removeAllMods(activeGame.id);
      await refreshMods(activeGame.id);
    });
  }

  function setTheme(theme: ThemePreference) {
    withBusy("Saving theme…", async () => {
      await api.setTheme(theme);
      await refreshSettings();
    });
  }

  const sortOptions = browseMode === "mods" ? MOD_SORTS : COLLECTION_SORTS;
  const downloadStatus = statusSummary(busy, assistQueue);

  function openDownloadsTab() {
    setTab("downloads");
    void refreshDownloads();
  }

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark">N</span>
          <div>
            <strong>Nexus Manager</strong>
            <small>Linux · MVP+</small>
          </div>
        </div>
        <nav>
          {(
            [
              ["setup", "Setup"],
              ["library", "Library"],
              ["browse", "Browse"],
              ["downloads", "Downloads"],
              ["settings", "Settings"],
            ] as const
          ).map(([id, label]) => (
            <button
              key={id}
              className={tab === id ? "nav active" : "nav"}
              onClick={() => {
                setTab(id);
                if (id === "downloads") refreshDownloads();
                if (id === "library" && detected.length === 0) scan();
              }}
            >
              {label}
            </button>
          ))}
        </nav>
        {activeGame ? (
          <div className="active-game">
            <span>Active</span>
            <strong>{activeGame.title}</strong>
            {downloadStatus && (
              <button
                type="button"
                className="active-game-status"
                onClick={openDownloadsTab}
                title="Open Downloads"
              >
                <span className="status-primary">{downloadStatus.primary}</span>
                {downloadStatus.secondary ? (
                  <span className="status-secondary">{downloadStatus.secondary}</span>
                ) : null}
              </button>
            )}
          </div>
        ) : (
          downloadStatus && (
            <button
              type="button"
              className="sidebar-status"
              onClick={openDownloadsTab}
              title="Open Downloads"
            >
              <span className="status-primary">{downloadStatus.primary}</span>
              {downloadStatus.secondary ? (
                <span className="status-secondary">{downloadStatus.secondary}</span>
              ) : null}
            </button>
          )
        )}
      </aside>

      <main className="main">
        {error && (
          <div className="banner error">
            <span>{error}</span>
            <span className="banner-actions">
              <button className="linkish" type="button" onClick={() => setError(null)}>
                dismiss
              </button>
            </span>
          </div>
        )}
        {notice && !error && (
          <div className={`banner ${notice.kind === "ok" ? "info" : "warn"}`}>
            <span>{notice.message}</span>
            <span className="banner-actions">
              <button className="linkish" type="button" onClick={() => setNotice(null)}>
                dismiss
              </button>
            </span>
          </div>
        )}

        {tab === "setup" && (
          <section className="panel">
            <h1>Setup</h1>
            <p>
              Paste your Nexus Mods personal API key from{" "}
              <a
                href="https://www.nexusmods.com/users/myaccount?tab=api"
                target="_blank"
                rel="noreferrer"
              >
                Account → API Access
              </a>
              . The API key authenticates browsing and Premium downloads. Free
              accounts also need a one-time Nexus website sign-in inside Download
              Assist (use Stay signed in); that session is saved in the app
              separately from the API key.
            </p>
            {settings?.user && (
              <div className="status-card">
                Signed in as <strong>{settings.user.name}</strong>
                {settings.user.is_premium ? (
                  <span className="pill ok">Premium</span>
                ) : (
                  <span className="pill warn">Free — Download Assist</span>
                )}
              </div>
            )}
            <div className="row">
              <input
                type="password"
                placeholder="Nexus API key"
                value={apiKeyInput}
                onChange={(e) => setApiKeyInput(e.target.value)}
              />
              <button onClick={saveApiKey} disabled={!apiKeyInput.trim()}>
                Save & validate
              </button>
            </div>
            {settings?.has_api_key && (
              <button
                className="danger"
                onClick={() =>
                  withBusy("Clearing key…", async () => {
                    await api.clearApiKey();
                    await refreshSettings();
                  })
                }
              >
                Clear API key
              </button>
            )}
          </section>
        )}

        {tab === "library" && (
          <section className="panel">
            {libraryDetail ? (
              <>
                <div className="panel-head">
                  <div className="detail-head-title">
                    <button type="button" onClick={closeLibraryDetail}>
                      ← Back
                    </button>
                    <h1>{libraryDetail.game.title}</h1>
                  </div>
                  <div className="panel-head-actions">
                    <div className="segment">
                      <button
                        className={libraryDetail.tab === "info" ? "active" : ""}
                        onClick={() => setGameDetailTab("info")}
                      >
                        Info
                      </button>
                      <button
                        className={libraryDetail.tab === "mods" ? "active" : ""}
                        onClick={() => setGameDetailTab("mods")}
                      >
                        Mods
                      </button>
                    </div>
                    <button
                      className="danger"
                      onClick={() =>
                        withBusy("Removing…", async () => {
                          await api.unmanageGame(libraryDetail.game.id);
                          await refreshManaged();
                        })
                      }
                    >
                      Unmanage
                    </button>
                  </div>
                </div>

                <div className="detail-hero">
                  <div className="detail-cover">
                    {mediaSrc(libraryDetail.game.cover_path) ? (
                      <img src={mediaSrc(libraryDetail.game.cover_path)!} alt="" />
                    ) : (
                      <PlaceholderIcon />
                    )}
                  </div>
                  <div className="detail-meta">
                    <p className="detail-author">
                      {shortLauncher(libraryDetail.game.launcher)}
                    </p>
                    <div className="detail-stats">
                      <span>{libraryDetail.game.nexus_domain}</span>
                      <span>{libraryDetail.game.plugin_id}</span>
                    </div>
                    {libraryDetail.tab === "mods" && (
                      <div className="detail-mod-actions">
                        <button onClick={importArchive}>Import archive</button>
                        <button onClick={deploy}>Deploy</button>
                        <button onClick={purge}>Purge</button>
                        <button
                          className="danger"
                          onClick={removeAllMods}
                          disabled={mods.length === 0}
                        >
                          Remove all mods
                        </button>
                      </div>
                    )}
                  </div>
                </div>

                {libraryDetail.tab === "info" ? (
                  <div className="detail-body">
                    <h2>About</h2>
                    <dl className="meta">
                      <dt>Launcher</dt>
                      <dd>{libraryDetail.game.launcher}</dd>
                      <dt>Domain</dt>
                      <dd>{libraryDetail.game.nexus_domain}</dd>
                      <dt>Install</dt>
                      <dd>{libraryDetail.game.install_path}</dd>
                      <dt>Plugin</dt>
                      <dd>{libraryDetail.game.plugin_id}</dd>
                    </dl>
                  </div>
                ) : (
                  <div className="detail-body">
                    <ul className="list mods">
                      {mods.map((m, i) => (
                        <li key={m.id}>
                          <label className="check">
                            <input
                              type="checkbox"
                              checked={m.enabled}
                              onChange={() => toggleMod(m)}
                            />
                            <div>
                              <strong>{m.name}</strong>
                              <small>
                                #{m.nexus_mod_id} · file {m.nexus_file_id}
                                {m.version ? ` · v${m.version}` : ""}
                              </small>
                            </div>
                          </label>
                          <div className="actions">
                            <button onClick={() => moveMod(i, -1)} disabled={i === 0}>
                              ↑
                            </button>
                            <button
                              onClick={() => moveMod(i, 1)}
                              disabled={i === mods.length - 1}
                            >
                              ↓
                            </button>
                            <button
                              className="danger"
                              onClick={() =>
                                withBusy("Removing mod…", async () => {
                                  if (!activeGame) return;
                                  await api.removeMod(activeGame.id, m.id);
                                  await refreshMods(activeGame.id);
                                })
                              }
                            >
                              Remove
                            </button>
                          </div>
                        </li>
                      ))}
                      {mods.length === 0 && (
                        <li className="empty">No staged mods. Browse Nexus to install some.</li>
                      )}
                    </ul>
                  </div>
                )}
              </>
            ) : (
              <>
                <div className="panel-head">
                  <h1>Library</h1>
                  <div className="panel-head-actions">
                    <div className="segment">
                      <button
                        className={libraryView === "list" ? "active" : ""}
                        onClick={() => setLibraryView("list")}
                      >
                        List
                      </button>
                      <button
                        className={libraryView === "grid" ? "active" : ""}
                        onClick={() => setLibraryView("grid")}
                      >
                        Grid
                      </button>
                    </div>
                    <button onClick={scan}>Scan games</button>
                  </div>
                </div>
                <p>
                  Supported plugins: Stardew Valley, Baldur&apos;s Gate 3, Cyberpunk
                  2077. Other detected games appear as unsupported.
                </p>
                {managed.length > 0 && (
                  <>
                    <h2>Managed</h2>
                    {libraryView === "grid" ? (
                      <div className="media-grid">
                        {managed.map((g) => (
                          <MediaCard
                            key={g.id}
                            title={g.title}
                            imageSrc={mediaSrc(g.cover_path)}
                            onClick={() => openGame(g, "mods")}
                          />
                        ))}
                      </div>
                    ) : (
                      <ul className="list">
                        {managed.map((g) => (
                          <li
                            key={g.id}
                            className="list-row-clickable"
                            onClick={() => openGame(g, "mods")}
                            onKeyDown={(e) => {
                              if (e.key === "Enter" || e.key === " ") {
                                e.preventDefault();
                                openGame(g, "mods");
                              }
                            }}
                            role="button"
                            tabIndex={0}
                          >
                            <div>
                              <strong>{g.title}</strong>
                              <small>
                                {g.launcher} · {g.nexus_domain}
                              </small>
                            </div>
                          </li>
                        ))}
                      </ul>
                    )}
                  </>
                )}
                <h2>Detected</h2>
                {libraryView === "grid" ? (
                  <div className="media-grid">
                    {detected.map((g) =>
                      g.supported ? (
                        <MediaCard
                          key={g.id}
                          title={g.title}
                          imageSrc={mediaSrc(g.cover_path)}
                          hoverLabel="Manage"
                          onClick={() => manage(g)}
                        />
                      ) : (
                        <MediaCard
                          key={g.id}
                          title={g.title}
                          imageSrc={mediaSrc(g.cover_path)}
                          coverLabel="Unsupported"
                          unsupported
                        />
                      ),
                    )}
                    {detected.length === 0 && (
                      <p className="note">Scan to find Steam / Heroic / Lutris / Bottles games.</p>
                    )}
                  </div>
                ) : (
                  <ul className="list">
                    {detected.map((g) => (
                      <li key={g.id}>
                        <div>
                          <strong>{g.title}</strong>
                          <small>
                            {g.launcher}
                            {g.install_path ? ` · ${g.install_path}` : ""}
                            {!g.supported ? " · unsupported" : ""}
                          </small>
                        </div>
                        {g.supported ? (
                          <button onClick={() => manage(g)}>Manage</button>
                        ) : null}
                      </li>
                    ))}
                    {detected.length === 0 && (
                      <li className="empty">Scan to find Steam / Heroic / Lutris / Bottles games.</li>
                    )}
                  </ul>
                )}
              </>
            )}
          </section>
        )}

        {tab === "browse" && (
          <section className="panel">
            {browseDetail ? (
              <>
                <div className="panel-head">
                  <div className="detail-head-title">
                    <button type="button" onClick={closeBrowseDetail}>
                      ← Back
                    </button>
                    <h1>{browseDetail.hit.name}</h1>
                  </div>
                  <div className="segment">
                    <button
                      className={browseDetail.tab === "info" ? "active" : ""}
                      onClick={() => setDetailTab("info")}
                    >
                      Info
                    </button>
                    <button
                      className={browseDetail.tab === "files" ? "active" : ""}
                      onClick={() => setDetailTab("files")}
                    >
                      Files
                    </button>
                  </div>
                </div>

                {browseDetail.kind === "mod" ? (
                  <>
                    <div className="detail-hero">
                      <div className="detail-cover">
                        {mediaSrc(
                          null,
                          modDetail?.picture_url ?? browseDetail.hit.picture_url,
                        ) ? (
                          <img
                            src={
                              mediaSrc(
                                null,
                                modDetail?.picture_url ?? browseDetail.hit.picture_url,
                              )!
                            }
                            alt=""
                          />
                        ) : (
                          <PlaceholderIcon />
                        )}
                      </div>
                      <div className="detail-meta">
                        <p className="detail-author">
                          {modDetail?.author ?? browseDetail.hit.author ?? "Unknown author"}
                        </p>
                        <div className="detail-stats">
                          {(modDetail?.endorsements ?? browseDetail.hit.endorsements) !=
                            null && (
                            <span>
                              {(
                                modDetail?.endorsements ?? browseDetail.hit.endorsements
                              )!.toLocaleString()}{" "}
                              endorsements
                            </span>
                          )}
                          {(modDetail?.downloads ?? browseDetail.hit.downloads) != null && (
                            <span>
                              {(
                                modDetail?.downloads ?? browseDetail.hit.downloads
                              )!.toLocaleString()}{" "}
                              downloads
                            </span>
                          )}
                          {modDetail?.version && <span>v{modDetail.version}</span>}
                          {(modDetail?.category ?? browseDetail.hit.category) && (
                            <span>{modDetail?.category ?? browseDetail.hit.category}</span>
                          )}
                        </div>
                        {browseDetail.hit.tags && browseDetail.hit.tags.length > 0 && (
                          <div className="media-card-tags">
                            {browseDetail.hit.tags.slice(0, 8).map((t) => (
                              <span key={t} className="pill">
                                {t}
                              </span>
                            ))}
                          </div>
                        )}
                        <dl className="meta detail-dates">
                          {formatTimestamp(modDetail?.created_timestamp) && (
                            <>
                              <dt>Created</dt>
                              <dd>{formatTimestamp(modDetail?.created_timestamp)}</dd>
                            </>
                          )}
                          {formatTimestamp(modDetail?.updated_timestamp) && (
                            <>
                              <dt>Updated</dt>
                              <dd>{formatTimestamp(modDetail?.updated_timestamp)}</dd>
                            </>
                          )}
                        </dl>
                      </div>
                    </div>

                    {browseDetail.tab === "info" ? (
                      <div className="detail-body">
                        <h2>About</h2>
                        <p className="detail-description">
                          {modDetail?.description
                            ? stripHtml(modDetail.description)
                            : modDetail?.summary ??
                              browseDetail.hit.summary ??
                              "No description available."}
                        </p>
                      </div>
                    ) : (
                      <div className="detail-body">
                        <h2>Files</h2>
                        <ul className="list">
                          {modFiles.map((f) => (
                            <li key={f.file_id}>
                              <div>
                                <strong>{f.name}</strong>
                                <small>
                                  {f.category_name ?? "file"}
                                  {f.version ? ` · v${f.version}` : ""}
                                  {f.is_primary ? " · primary" : ""}
                                  {f.size_kb != null
                                    ? ` · ${(f.size_kb / 1024).toFixed(1)} MB`
                                    : ""}
                                </small>
                              </div>
                              <button onClick={() => installFile(f)}>
                                {isPremium ? "Install" : "Download Assist"}
                              </button>
                            </li>
                          ))}
                          {modFiles.length === 0 && (
                            <li className="empty">No files found for this mod.</li>
                          )}
                        </ul>
                      </div>
                    )}
                  </>
                ) : (
                  <>
                    <div className="detail-hero">
                      <div className="detail-cover">
                        {mediaSrc(null, browseDetail.hit.tile_image_url) ? (
                          <img
                            src={mediaSrc(null, browseDetail.hit.tile_image_url)!}
                            alt=""
                          />
                        ) : (
                          <PlaceholderIcon />
                        )}
                      </div>
                      <div className="detail-meta">
                        <p className="detail-author">Collection</p>
                        <div className="detail-stats">
                          {browseDetail.hit.endorsements != null && (
                            <span>
                              {browseDetail.hit.endorsements.toLocaleString()} endorsements
                            </span>
                          )}
                          {browseDetail.hit.total_downloads != null && (
                            <span>
                              {browseDetail.hit.total_downloads.toLocaleString()} downloads
                            </span>
                          )}
                          {browseDetail.hit.revision_number != null && (
                            <span>rev {browseDetail.hit.revision_number}</span>
                          )}
                          {browseDetail.hit.domain_name && (
                            <span>{browseDetail.hit.domain_name}</span>
                          )}
                        </div>
                        <div className="actions">
                          <button onClick={() => installCollection(browseDetail.hit)}>
                            {isPremium ? "Install" : "Download Assist"}
                          </button>
                        </div>
                        <label className="inline">
                          <input
                            type="checkbox"
                            checked={includeOptional}
                            onChange={(e) => setIncludeOptional(e.target.checked)}
                          />
                          Include optional collection mods
                        </label>
                      </div>
                    </div>

                    {browseDetail.tab === "info" ? (
                      <div className="detail-body">
                        <h2>About</h2>
                        <p className="detail-description">
                          {browseDetail.hit.summary ?? "No description available."}
                        </p>
                      </div>
                    ) : (
                      <div className="detail-body">
                        <h2>Included mods</h2>
                        <ul className="list">
                          {groupCollectionMods(collectionModFiles).map((group) => {
                            const primary = group.files[0];
                            const extras =
                              group.files.length > 1
                                ? ` · ${group.files.length} files`
                                : "";
                            return (
                              <li
                                key={group.mod_id}
                                className="list-row-clickable"
                                onClick={() =>
                                  openMod(
                                    {
                                      mod_id: group.mod_id,
                                      name: group.mod_name,
                                      summary: null,
                                      picture_url: null,
                                      downloads: null,
                                      endorsements: null,
                                      author: null,
                                      domain_name: group.domain_name,
                                    },
                                    "info",
                                  )
                                }
                              >
                                <div>
                                  <strong>{group.mod_name}</strong>
                                  <small>
                                    {primary.file_name}
                                    {primary.version ? ` · v${primary.version}` : ""}
                                    {primary.optional ? " · optional" : ""}
                                    {extras}
                                  </small>
                                </div>
                              </li>
                            );
                          })}
                          {collectionModFiles.length === 0 && (
                            <li className="empty">No mods found in this collection.</li>
                          )}
                        </ul>
                      </div>
                    )}
                  </>
                )}
              </>
            ) : (
              <>
                <div className="panel-head">
                  <h1>Browse Nexus</h1>
                  <div className="panel-head-actions">
                    <div className="segment">
                      <button
                        className={browseMode === "mods" ? "active" : ""}
                        onClick={() => {
                          setBrowseMode("mods");
                          setBrowseSort("endorsements");
                        }}
                      >
                        Mods
                      </button>
                      <button
                        className={browseMode === "collections" ? "active" : ""}
                        onClick={() => {
                          setBrowseMode("collections");
                          setBrowseSort("endorsements");
                        }}
                      >
                        Collections
                      </button>
                    </div>
                    <div className="segment">
                      <button
                        className={browseView === "list" ? "active" : ""}
                        onClick={() => setBrowseView("list")}
                      >
                        List
                      </button>
                      <button
                        className={browseView === "grid" ? "active" : ""}
                        onClick={() => setBrowseView("grid")}
                      >
                        Grid
                      </button>
                    </div>
                  </div>
                </div>
                {!activeGame ? (
                  <p>Select a managed game first.</p>
                ) : (
                  <>
                    <div className="row">
                      <input
                        placeholder={`Search ${activeGame.nexus_domain}…`}
                        value={searchQuery}
                        onChange={(e) => setSearchQuery(e.target.value)}
                        onKeyDown={(e) => e.key === "Enter" && search()}
                      />
                      <button onClick={search}>Search</button>
                    </div>
                    <div className="filters">
                      <label>
                        Sort
                        <select
                          value={browseSort}
                          onChange={(e) => {
                            const sort = e.target.value;
                            setBrowseSort(sort);
                            runSearch(browseMode, searchQuery, {
                              ...browseOpts,
                              sort,
                            });
                          }}
                        >
                          {sortOptions.map((o) => (
                            <option key={o.value} value={o.value}>
                              {o.label}
                            </option>
                          ))}
                        </select>
                      </label>
                      <label>
                        Category
                        <select
                          value={browseCategory}
                          onChange={(e) => {
                            const category = e.target.value;
                            setBrowseCategory(category);
                            runSearch(browseMode, searchQuery, {
                              ...browseOpts,
                              category: category || null,
                            });
                          }}
                        >
                          <option value="">Any</option>
                          {(browseMeta?.categories ?? []).map((c) => (
                            <option key={c} value={c}>
                              {c}
                            </option>
                          ))}
                        </select>
                      </label>
                      <label>
                        Game version
                        <select
                          value={browseVersion}
                          onChange={(e) => {
                            const gameVersion = e.target.value;
                            setBrowseVersion(gameVersion);
                            runSearch(browseMode, searchQuery, {
                              ...browseOpts,
                              gameVersion: gameVersion || null,
                            });
                          }}
                        >
                          <option value="">Any</option>
                          {(browseMeta?.game_versions ?? []).map((v) => (
                            <option key={v} value={v}>
                              {v}
                            </option>
                          ))}
                        </select>
                      </label>
                      <div className="tag-picker" ref={tagPickerRef}>
                        <span className="tag-picker-label" id="browse-tags-label">
                          Tags
                        </span>
                        <button
                          type="button"
                          className="tag-picker-trigger"
                          aria-expanded={tagsOpen}
                          aria-haspopup="listbox"
                          aria-labelledby="browse-tags-label"
                          onClick={() => setTagsOpen((open) => !open)}
                        >
                          {browseTags.length === 0
                            ? "Any"
                            : browseTags.length === 1
                              ? browseTags[0]
                              : `${browseTags.length} tags`}
                        </button>
                        {tagsOpen && (
                          <div
                            className="tag-picker-popup"
                            role="listbox"
                            aria-multiselectable="true"
                            aria-labelledby="browse-tags-label"
                          >
                            <input
                              ref={tagSearchRef}
                              type="text"
                              className="tag-picker-search"
                              placeholder="Filter tags…"
                              value={tagQuery}
                              onChange={(e) => setTagQuery(e.target.value)}
                              onKeyDown={(e) => {
                                if (e.key === "Escape") {
                                  e.stopPropagation();
                                  setTagsOpen(false);
                                }
                              }}
                            />
                            <div className="tag-picker-list">
                              {orderedTags.length === 0 ? (
                                <div className="tag-picker-empty">No tags</div>
                              ) : (
                                orderedTags.map((t) => (
                                  <label key={t} className="tag-picker-item">
                                    <input
                                      type="checkbox"
                                      checked={browseTags.includes(t)}
                                      onChange={() => toggleBrowseTag(t)}
                                    />
                                    <span>{t}</span>
                                  </label>
                                ))
                              )}
                            </div>
                          </div>
                        )}
                      </div>
                      <button
                        type="button"
                        className="filters-clear"
                        onClick={clearBrowseFilters}
                      >
                        Clear filters
                      </button>
                    </div>
                    {browseMode === "collections" && (
                      <label className="inline">
                        <input
                          type="checkbox"
                          checked={includeOptional}
                          onChange={(e) => setIncludeOptional(e.target.checked)}
                        />
                        Include optional collection mods
                      </label>
                    )}
                    {browseMode === "mods" ? (
                      browseView === "grid" ? (
                        <div className="media-grid">
                          {modHits.map((m) => {
                            const tags = [
                              ...(m.category ? [m.category] : []),
                              ...(m.tags ?? []).slice(0, 3),
                            ];
                            return (
                              <MediaCard
                                key={m.mod_id}
                                title={m.name}
                                imageSrc={mediaSrc(null, m.picture_url)}
                                badge={m.author ?? "Mod"}
                                overlay={
                                  m.endorsements != null
                                    ? `${m.endorsements.toLocaleString()} endorsements`
                                    : null
                                }
                                tags={tags}
                                onClick={() => openMod(m, "info")}
                                actions={
                                  <button
                                    onClick={(e) => {
                                      e.stopPropagation();
                                      openMod(m, "files");
                                    }}
                                  >
                                    Files
                                  </button>
                                }
                              />
                            );
                          })}
                          {modHits.length === 0 && (
                            <p className="note">Search to find mods for this game.</p>
                          )}
                        </div>
                      ) : (
                        <ul className="list">
                          {modHits.map((m) => (
                            <li key={m.mod_id} className="list-row-clickable">
                              <div
                                className="list-row-main"
                                onClick={() => openMod(m, "info")}
                                onKeyDown={(e) => {
                                  if (e.key === "Enter" || e.key === " ") {
                                    e.preventDefault();
                                    openMod(m, "info");
                                  }
                                }}
                                role="button"
                                tabIndex={0}
                              >
                                <strong>{m.name}</strong>
                                <small>
                                  {m.author ?? "unknown"}
                                  {m.endorsements != null
                                    ? ` · ${m.endorsements} endorsements`
                                    : ""}
                                </small>
                                {m.summary && <p className="summary">{m.summary}</p>}
                              </div>
                              <button onClick={() => openMod(m, "files")}>Files</button>
                            </li>
                          ))}
                        </ul>
                      )
                    ) : browseView === "grid" ? (
                      <div className="media-grid">
                        {collectionHits.map((c) => {
                          const tags = [
                            ...(c.revision_number != null
                              ? [`rev ${c.revision_number}`]
                              : []),
                            ...(c.domain_name ? [c.domain_name] : []),
                          ];
                          return (
                            <MediaCard
                              key={c.slug}
                              title={c.name}
                              imageSrc={mediaSrc(null, c.tile_image_url)}
                              badge="Collection"
                              overlay={
                                c.endorsements != null
                                  ? `${c.endorsements.toLocaleString()} endorsements`
                                  : c.total_downloads != null
                                    ? `${c.total_downloads.toLocaleString()} downloads`
                                    : null
                              }
                              tags={tags}
                              onClick={() => openCollection(c, "info")}
                              actions={
                                <button
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    installCollection(c);
                                  }}
                                >
                                  Install
                                </button>
                              }
                            />
                          );
                        })}
                        {collectionHits.length === 0 && (
                          <p className="note">Search to find collections for this game.</p>
                        )}
                      </div>
                    ) : (
                      <ul className="list">
                        {collectionHits.map((c) => (
                          <li key={c.slug} className="list-row-clickable">
                            <div
                              className="list-row-main"
                              onClick={() => openCollection(c, "info")}
                              onKeyDown={(e) => {
                                if (e.key === "Enter" || e.key === " ") {
                                  e.preventDefault();
                                  openCollection(c, "info");
                                }
                              }}
                              role="button"
                              tabIndex={0}
                            >
                              <strong>{c.name}</strong>
                              <small>{c.slug}</small>
                              {c.summary && <p className="summary">{c.summary}</p>}
                            </div>
                            <button onClick={() => installCollection(c)}>Install</button>
                          </li>
                        ))}
                      </ul>
                    )}
                  </>
                )}
              </>
            )}
          </section>
        )}

        {tab === "downloads" && (
          <DownloadsWorkspace
            downloads={downloads}
            queue={assistQueue}
            activeBatch={activeBatch}
            assistActive={assistActive}
            assistHint={assistHint}
            onRefresh={refreshDownloads}
            onSkip={() => skipAssistCurrent()}
            onCancel={() => cancelAssistQueue()}
            onCancelRemaining={() => cancelRemainingBatch()}
            onCancelDownload={(id) => cancelOneDownload(id)}
            onAssistHost={handleAssistHost}
          />
        )}

        {tab === "settings" && (
          <section className="panel">
            <h1>Settings</h1>
            <div className="setting-block">
              <span>Theme</span>
              <div className="segment">
                {(["light", "dark", "system"] as const).map((t) => (
                  <button
                    key={t}
                    className={themePref === t ? "active" : ""}
                    onClick={() => setTheme(t)}
                  >
                    {t === "light" ? "Light" : t === "dark" ? "Dark" : "System"}
                  </button>
                ))}
              </div>
            </div>
            <label className="inline">
              <input
                type="checkbox"
                checked={settings?.adult_content ?? true}
                onChange={(e) =>
                  withBusy("Saving…", async () => {
                    await api.setAdultContent(e.target.checked);
                    await refreshSettings();
                  })
                }
              />
              Show adult content in search
            </label>
            <label className="inline">
              <input
                type="checkbox"
                checked={settings?.autoclick_free_download ?? true}
                onChange={(e) =>
                  withBusy("Saving…", async () => {
                    await api.setAutoclickFreeDownload(e.target.checked);
                    await refreshSettings();
                  })
                }
              />
              Autoclick free download button in Download Assist (Nexus Fast
              Download–style; waits for countdown)
            </label>
            <button
              type="button"
              onClick={() =>
                withBusy("Clearing Nexus website session…", async () => {
                  await api.clearAssistSession();
                })
              }
            >
              Clear Nexus website session
            </button>
            {settings && (
              <dl className="meta">
                <dt>Config</dt>
                <dd>{settings.config_dir}</dd>
                <dt>Data</dt>
                <dd>{settings.data_dir}</dd>
                <dt>Cache</dt>
                <dd>{settings.cache_dir}</dd>
              </dl>
            )}
            <p className="note">
              Free accounts: Install opens the Downloads workspace with an embedded
              Nexus Assist panel. Sign in there once with Stay signed in; that session
              is saved separately from your API key. Autoclick can press Slow download
              for you. Premium: one-click Install and Collections. Register{" "}
              <code>nxm://</code> via the desktop entry for browser “Download with
              manager” links.
            </p>
          </section>
        )}
      </main>
    </div>
  );
}

export default App;
