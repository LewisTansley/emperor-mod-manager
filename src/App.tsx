import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "./api";
import {
  createQueueEntry,
  DownloadsWorkspace,
  formatBytes,
  type ActiveDownloadBatch,
  type AssistQueueEntry,
  type AssistQueueState,
} from "./downloads";
import { RichText } from "./RichText";
import type {
  BrowseMeta,
  BrowseSearchOpts,
  CatalogHit,
  CatalogSuggestion,
  CollectionDetail,
  CollectionHit,
  CollectionModFile,
  DetectedGame,
  DownloadItem,
  GameInfo,
  ManagedGame,
  ModDetail,
  ModFileInfo,
  Settings,
  StagedMod,
  TagFilterState,
  ThemePreference,
  TsPackageDetail,
  ModioModDetail,
  ModioFileInfo,
} from "./types";
import "./App.css";

type Tab = "setup" | "library" | "browse" | "downloads" | "settings";
type ViewMode = "list" | "grid";
type DetailTab = "info" | "files";
type GameDetailTab = "info" | "mods";
type BrowseTagMap = Record<string, TagFilterState>;
type CatalogSourceFilter = "all" | "nexus" | "thunderstore" | "modio";

type StatusNotice = {
  kind: "ok" | "warn";
  message: string;
};

type BrowseDetail =
  | { kind: "mod"; hit: CatalogHit; tab: DetailTab }
  | { kind: "collection"; hit: CollectionHit; tab: DetailTab };

/** Cycle: unset → include → exclude → unset */
function cycleTagState(current: TagFilterState | undefined): TagFilterState | undefined {
  if (!current) return "include";
  if (current === "include") return "exclude";
  return undefined;
}

function tagListsFromMap(map: BrowseTagMap): {
  tagsInclude: string[];
  tagsExclude: string[];
} {
  const tagsInclude: string[] = [];
  const tagsExclude: string[] = [];
  for (const [tag, state] of Object.entries(map)) {
    if (state === "include") tagsInclude.push(tag);
    else if (state === "exclude") tagsExclude.push(tag);
  }
  return { tagsInclude, tagsExclude };
}

function setTagInMap(map: BrowseTagMap, tag: string): BrowseTagMap {
  const next = { ...map };
  const cycled = cycleTagState(map[tag]);
  if (!cycled) delete next[tag];
  else next[tag] = cycled;
  return next;
}

type StatusSummary = {
  primary: string;
  secondary: string;
};

function statusSummary(
  busy: string | null,
  assistQueue: AssistQueueState | null,
  downloads: DownloadItem[],
  activeBatch: ActiveDownloadBatch | null,
): StatusSummary | null {
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

  if (busy) {
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

  const inFlight = downloads.filter(
    (d) => d.status === "downloading" ||
              d.status === "extracting" ||
              d.status === "paused",
  );
  if (inFlight.length === 0) return null;

  let secondary: string;
  if (inFlight.length === 1) {
    secondary = inFlight[0].label;
  } else if (activeBatch?.label) {
    secondary = activeBatch.label;
  } else {
    secondary = `${inFlight.length} remaining`;
  }
  return { primary: "Downloading", secondary };
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

function openExternal(url: string) {
  void (async () => {
    try {
      await openUrl(url);
    } catch (e) {
      console.error("Failed to open URL", e);
    }
  })();
}

function formatTimestamp(ts: number | null | undefined): string | null {
  if (ts == null || ts <= 0) return null;
  try {
    return new Date(ts * 1000).toLocaleDateString();
  } catch {
    return null;
  }
}

function formatIsoDate(iso: string | null | undefined): string | null {
  if (!iso) return null;
  try {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return null;
    return d.toLocaleDateString();
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
  const [modHits, setModHits] = useState<CatalogHit[]>([]);
  const [collectionHits, setCollectionHits] = useState<CollectionHit[]>([]);
  const [browseMode, setBrowseMode] = useState<"mods" | "collections">("mods");
  const [sourceFilter, setSourceFilter] = useState<CatalogSourceFilter>("all");
  const [libraryView, setLibraryView] = useState<ViewMode>("grid");
  const [browseView, setBrowseView] = useState<ViewMode>("grid");
  const [browseSort, setBrowseSort] = useState("endorsements");
  const [browseCategory, setBrowseCategory] = useState("");
  const [browseVersion, setBrowseVersion] = useState("");
  const [browseTags, setBrowseTags] = useState<BrowseTagMap>({});
  const [tagQuery, setTagQuery] = useState("");
  const [tagsOpen, setTagsOpen] = useState(false);
  const [browseTotalCount, setBrowseTotalCount] = useState(0);
  const [browseHasMore, setBrowseHasMore] = useState(false);
  const [browseSearched, setBrowseSearched] = useState(false);
  const [browseMeta, setBrowseMeta] = useState<BrowseMeta | null>(null);
  const [browseDetail, setBrowseDetail] = useState<BrowseDetail | null>(null);
  const [libraryDetail, setLibraryDetail] = useState<{
    game: ManagedGame;
    tab: GameDetailTab;
  } | null>(null);
  const [gameInfo, setGameInfo] = useState<GameInfo | null>(null);
  const [modDetail, setModDetail] = useState<ModDetail | null>(null);
  const [tsDetail, setTsDetail] = useState<TsPackageDetail | null>(null);
  const [tsVersion, setTsVersion] = useState("");
  const [modioDetail, setModioDetail] = useState<ModioModDetail | null>(null);
  const [modioFiles, setModioFiles] = useState<ModioFileInfo[]>([]);
  const [modFiles, setModFiles] = useState<ModFileInfo[]>([]);
  const [modioKeyInput, setModioKeyInput] = useState("");
  const [modioGameIdEdit, setModioGameIdEdit] = useState("");
  const [collectionModFiles, setCollectionModFiles] = useState<CollectionModFile[]>([]);
  const [collectionDetail, setCollectionDetail] = useState<CollectionDetail | null>(null);
  const [includeOptional, setIncludeOptional] = useState(false);
  const [assistQueue, setAssistQueue] = useState<AssistQueueState | null>(null);
  const [activeBatch, setActiveBatch] = useState<ActiveDownloadBatch | null>(null);
  const [assistHint, setAssistHint] = useState<string | null>(null);
  const [assistActive, setAssistActive] = useState(false);
  const [unrealManage, setUnrealManage] = useState<DetectedGame | null>(null);
  const [unrealDomain, setUnrealDomain] = useState("");
  const [unrealCommunity, setUnrealCommunity] = useState("");
  const [unrealModioId, setUnrealModioId] = useState("");
  const [unrealProject, setUnrealProject] = useState("");
  const [bepinexManage, setBepinexManage] = useState<DetectedGame | null>(null);
  const [bepinexDomain, setBepinexDomain] = useState("");
  const [bepinexCommunity, setBepinexCommunity] = useState("");
  const [bepinexModioId, setBepinexModioId] = useState("");
  const [catalogHints, setCatalogHints] = useState<CatalogSuggestion | null>(
    null,
  );
  const catalogSuggestGen = useRef(0);
  const [ueProjectEdit, setUeProjectEdit] = useState("");
  const [ueDomainEdit, setUeDomainEdit] = useState("");
  const [tsCommunityEdit, setTsCommunityEdit] = useState("");
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
  const mainScrollRef = useRef<HTMLElement | null>(null);
  const browseSentinelRef = useRef<HTMLDivElement | null>(null);
  const browseLoadingMoreRef = useRef(false);
  const browseNextOffsetRef = useRef(0);
  const browseAppendFailedRef = useRef(false);
  const browseRequestIdRef = useRef(0);
  const browseModeRef = useRef(browseMode);
  browseModeRef.current = browseMode;
  const searchQueryRef = useRef(searchQuery);
  searchQueryRef.current = searchQuery;
  const browseHasMoreRef = useRef(browseHasMore);
  browseHasMoreRef.current = browseHasMore;
  const browseOptsRef = useRef<BrowseSearchOpts>({
    sort: browseSort,
    category: browseCategory || null,
    tagsInclude: [],
    tagsExclude: [],
    gameVersion: browseVersion || null,
    offset: 0,
  });

  const activeGame = useMemo(
    () => managed.find((g) => g.id === activeId) ?? managed[0] ?? null,
    [managed, activeId],
  );
  const activeGameRef = useRef(activeGame);
  activeGameRef.current = activeGame;

  const unmanaged = useMemo(() => {
    const managedIds = new Set(managed.map((g) => g.id));
    return detected.filter((g) => !managedIds.has(g.id));
  }, [detected, managed]);

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

  const scan = useCallback(async () => {
    setBusy("Scanning installed games…");
    setError(null);
    setNotice(null);
    try {
      setDetected(await api.scanGames());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }, []);

  useEffect(() => {
    (async () => {
      try {
        const s = await refreshSettings();
        await refreshManaged();
        await scan();
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
  }, [refreshManaged, refreshSettings, scan]);

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
      setCollectionDetail(null);
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
    if (!activeGame.nexus_domain) {
      setBrowseMeta(null);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const meta = await api.browseMeta(activeGame.nexus_domain);
        if (!cancelled) setBrowseMeta(meta);
      } catch (e) {
        if (!cancelled) setBrowseMeta(null);
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

  useEffect(() => {
    // List (and sentinel) unmount while browseDetail is open — re-bind on Back.
    if (tab !== "browse" || browseDetail || !browseHasMore) return;
    const sentinel = browseSentinelRef.current;
    const root = mainScrollRef.current;
    if (!sentinel || !root) return;

    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries.some((e) => e.isIntersecting);
        if (!visible) {
          browseAppendFailedRef.current = false;
          return;
        }
        if (
          browseLoadingMoreRef.current ||
          !browseHasMoreRef.current ||
          browseAppendFailedRef.current
        ) {
          return;
        }
        void runSearch(
          browseModeRef.current,
          searchQueryRef.current,
          browseOptsRef.current,
          true,
        );
      },
      { root, rootMargin: "240px", threshold: 0 },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
    // Re-bind when list remounts / size / mode / view changes so the sentinel stays observed.
    // eslint-disable-next-line react-hooks/exhaustive-deps -- runSearch reads latest via refs
  }, [
    tab,
    browseDetail,
    browseMode,
    browseView,
    browseHasMore,
    modHits.length,
    collectionHits.length,
  ]);

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
          (d.status === "downloading" ||
              d.status === "extracting" ||
              d.status === "paused"),
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

  const pauseOneDownload = useCallback(
    async (id: string) => {
      try {
        await api.pauseDownload(id);
      } catch (e) {
        setError(String(e));
      }
      await refreshDownloads();
    },
    [refreshDownloads],
  );

  const resumeOneDownload = useCallback(
    async (id: string) => {
      try {
        await api.resumeDownload(id);
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
              (d.status === "downloading" ||
              d.status === "extracting" ||
              d.status === "paused"),
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
              (d.status === "downloading" ||
              d.status === "extracting" ||
              d.status === "paused"),
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
              (d.status === "downloading" ||
              d.status === "extracting" ||
              d.status === "paused"),
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

  async function saveModioApiKey() {
    await withBusy("Validating mod.io API key…", async () => {
      await api.setModioApiKey(modioKeyInput);
      setModioKeyInput("");
      await refreshSettings();
      setNotice({ kind: "ok", message: "mod.io API key saved." });
    });
  }

  async function manage(game: DetectedGame) {
    if (!game.supported || !game.plugin_id || !game.install_path) {
      setError("This game is not supported yet.");
      return;
    }
    if (!game.nexus_domain && game.engine_hint !== "bepinex") {
      setError("This game is not supported yet.");
      return;
    }
    await withBusy(`Managing ${game.title}…`, async () => {
      await api.manageGame({
        id: game.id,
        title: game.title,
        nexusDomain: game.nexus_domain ?? "",
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

  function beginUnrealManage(game: DetectedGame) {
    if (!game.install_path) {
      setError("Install path is required to manage this Unreal game.");
      return;
    }
    setError(null);
    setBepinexManage(null);
    setUnrealManage(game);
    setUnrealDomain("");
    setUnrealCommunity("");
    setUnrealModioId("");
    setUnrealProject("");
    setCatalogHints(null);
    void api.detectUeLayout(game.install_path).then((layout) => {
      if (layout?.project_name) {
        setUnrealProject(layout.project_name);
      }
    });
    const gen = ++catalogSuggestGen.current;
    void withBusy("Looking up catalogs…", async () => {
      try {
        const s = await api.suggestCatalogIds(game.title);
        if (catalogSuggestGen.current !== gen) {
          return;
        }
        if (s.nexus_domain) {
          setUnrealDomain(s.nexus_domain);
        }
        if (s.thunderstore_community) {
          setUnrealCommunity(s.thunderstore_community);
        }
        if (s.modio_game_id != null) {
          setUnrealModioId(String(s.modio_game_id));
        }
        setCatalogHints(s);
      } catch {
        // Leave fields empty for manual entry.
      }
    });
  }

  function beginBepinexManage(game: DetectedGame) {
    if (!game.install_path) {
      setError("Install path is required to manage this Unity game.");
      return;
    }
    setError(null);
    setUnrealManage(null);
    setBepinexManage(game);
    setBepinexDomain(game.nexus_domain ?? "");
    setBepinexCommunity("");
    setBepinexModioId("");
    setCatalogHints(null);
    const gen = ++catalogSuggestGen.current;
    void withBusy("Looking up catalogs…", async () => {
      try {
        const s = await api.suggestCatalogIds(game.title);
        if (catalogSuggestGen.current !== gen) {
          return;
        }
        if (s.nexus_domain) {
          setBepinexDomain(s.nexus_domain);
        } else if (!game.nexus_domain) {
          setBepinexDomain("");
        }
        if (s.thunderstore_community) {
          setBepinexCommunity(s.thunderstore_community);
        }
        if (s.modio_game_id != null) {
          setBepinexModioId(String(s.modio_game_id));
        }
        setCatalogHints(s);
      } catch {
        // Leave fields empty for manual entry.
      }
    });
  }

  async function confirmUnrealManage() {
    const game = unrealManage;
    if (!game?.install_path) {
      return;
    }
    const domain = unrealDomain.trim().toLowerCase().replace(/\s+/g, "");
    const community = unrealCommunity.trim().toLowerCase();
    const midRaw = unrealModioId.trim();
    const modioGameId = midRaw ? Number.parseInt(midRaw, 10) : 0;
    if (!domain && !community && !(Number.isFinite(modioGameId) && modioGameId > 0)) {
      setError(
        "Enter a Nexus Mods domain, Thunderstore community, and/or mod.io game ID.",
      );
      return;
    }
    await withBusy(`Managing ${game.title}…`, async () => {
      await api.manageGame({
        id: game.id,
        title: game.title,
        nexusDomain: domain,
        installPath: game.install_path!,
        launcher: game.launcher,
        pluginId: "unreal",
        coverPath: game.cover_path,
        projectName: unrealProject.trim() || null,
        thunderstoreCommunity: community || null,
        modioGameId: Number.isFinite(modioGameId) && modioGameId > 0 ? modioGameId : null,
      });
      setUnrealManage(null);
      setCatalogHints(null);
      const list = await refreshManaged();
      const managedGame = list.find((g) => g.id === game.id);
      if (managedGame) {
        await openGame(managedGame, "mods");
      } else {
        setActiveId(game.id);
      }
    });
  }

  async function confirmBepinexManage() {
    const game = bepinexManage;
    if (!game?.install_path) {
      return;
    }
    const domain = bepinexDomain.trim().toLowerCase().replace(/\s+/g, "");
    const community = bepinexCommunity.trim().toLowerCase();
    const midRaw = bepinexModioId.trim();
    const modioGameId = midRaw ? Number.parseInt(midRaw, 10) : 0;
    if (!domain && !community && !(Number.isFinite(modioGameId) && modioGameId > 0)) {
      setError(
        "Enter a Nexus Mods domain, Thunderstore community, and/or mod.io game ID.",
      );
      return;
    }
    await withBusy(`Managing ${game.title}…`, async () => {
      await api.manageGame({
        id: game.id,
        title: game.title,
        nexusDomain: domain,
        installPath: game.install_path!,
        launcher: game.launcher,
        pluginId: "bepinex",
        coverPath: game.cover_path,
        thunderstoreCommunity: community || null,
        modioGameId: Number.isFinite(modioGameId) && modioGameId > 0 ? modioGameId : null,
      });
      setBepinexManage(null);
      setCatalogHints(null);
      const list = await refreshManaged();
      const managedGame = list.find((g) => g.id === game.id);
      if (managedGame) {
        await openGame(managedGame, "mods");
      } else {
        setActiveId(game.id);
      }
    });
  }

  async function saveUeGameSettings() {
    if (!libraryDetail) {
      return;
    }
    await withBusy("Saving game settings…", async () => {
      const midRaw = modioGameIdEdit.trim();
      const modioGameId = midRaw ? Number.parseInt(midRaw, 10) : 0;
      const updated = await api.updateManagedGame({
        id: libraryDetail.game.id,
        nexusDomain: ueDomainEdit.trim() || null,
        projectName: ueProjectEdit.trim(),
        thunderstoreCommunity: tsCommunityEdit.trim() || null,
        modioGameId: Number.isFinite(modioGameId) ? modioGameId : 0,
      });
      setLibraryDetail({ ...libraryDetail, game: updated });
      await refreshManaged();
      setNotice({ kind: "ok", message: "Game settings saved." });
    });
  }

  const browseOpts = useMemo((): BrowseSearchOpts => {
    const { tagsInclude, tagsExclude } = tagListsFromMap(browseTags);
    return {
      sort: browseSort,
      category: browseCategory || null,
      tagsInclude,
      tagsExclude,
      gameVersion: browseVersion || null,
      offset: 0,
    };
  }, [browseSort, browseCategory, browseTags, browseVersion]);

  browseOptsRef.current = browseOpts;

  useEffect(() => {
    if (tab !== "browse" || browseMode !== "mods" || !browseSearched) return;
    void runSearch("mods", searchQuery, { ...browseOpts, offset: 0 });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sourceFilter]);

  const activeTagCount = useMemo(
    () => Object.keys(browseTags).length,
    [browseTags],
  );

  const orderedTags = useMemo(() => {
    const all =
      browseMode === "mods"
        ? (browseMeta?.mod_tags ?? [])
        : (browseMeta?.collection_tags ?? []);
    const q = tagQuery.trim().toLowerCase();
    const filtered = q ? all.filter((t) => t.toLowerCase().includes(q)) : all;
    return [...filtered].sort((a, b) => {
      const rank = (t: string) =>
        browseTags[t] === "include" ? 0 : browseTags[t] === "exclude" ? 1 : 2;
      const ar = rank(a);
      const br = rank(b);
      if (ar !== br) return ar - br;
      return a.localeCompare(b, undefined, { sensitivity: "base" });
    });
  }, [browseMeta?.mod_tags, browseMeta?.collection_tags, browseMode, browseTags, tagQuery]);

  async function runSearch(
    mode: "mods" | "collections" = browseMode,
    query = searchQuery,
    opts: BrowseSearchOpts = browseOpts,
    append = false,
  ) {
    if (!activeGame) {
      setError("Manage a game first.");
      return;
    }
    const hasNexus = Boolean(activeGame.nexus_domain);
    const hasTs = Boolean(activeGame.thunderstore_community);
    const hasModio = Boolean(activeGame.modio_game_id);
    if (mode === "collections" && !hasNexus) {
      setError("Collections require a Nexus Mods domain for this game.");
      return;
    }
    if (mode === "mods" && !hasNexus && !hasTs && !hasModio) {
      setError(
        "Set a Nexus domain, Thunderstore community, and/or mod.io game ID for this game.",
      );
      return;
    }
    if (append) {
      if (
        browseLoadingMoreRef.current ||
        !browseHasMoreRef.current ||
        browseAppendFailedRef.current
      ) {
        return;
      }
      browseLoadingMoreRef.current = true;
    }

    const requestId = ++browseRequestIdRef.current;
    const offset = append ? browseNextOffsetRef.current : 0;
    const requestOpts: BrowseSearchOpts = { ...opts, offset };
    let appendOk = false;

    const applyModPage = (
      items: CatalogHit[],
      totalCount: number,
      hasMore: boolean,
      nextOffset: number,
      merge: "replace" | "append",
    ) => {
      browseNextOffsetRef.current = nextOffset;
      browseHasMoreRef.current = hasMore;
      setBrowseHasMore(hasMore);
      setBrowseTotalCount(totalCount);
      if (merge === "replace") {
        setModHits(items);
        setBrowseSearched(true);
      } else {
        setModHits((prev) => {
          const seen = new Set(prev.map((m) => m.id));
          const merged = [...prev];
          for (const item of items) {
            if (!seen.has(item.id)) {
              seen.add(item.id);
              merged.push(item);
            }
          }
          return merged;
        });
      }
    };

    const applyCollectionPage = (
      items: CollectionHit[],
      totalCount: number,
      merge: "replace" | "append",
    ) => {
      const nextOffset = offset + items.length;
      browseNextOffsetRef.current = nextOffset;
      const hasMore = items.length > 0 && nextOffset < totalCount;
      browseHasMoreRef.current = hasMore;
      setBrowseHasMore(hasMore);
      setBrowseTotalCount(totalCount);
      if (merge === "replace") {
        setCollectionHits(items);
        setBrowseSearched(true);
      } else {
        setCollectionHits((prev) => {
          const seen = new Set(prev.map((c) => c.slug));
          const merged = [...prev];
          for (const item of items) {
            if (!seen.has(item.slug)) {
              seen.add(item.slug);
              merged.push(item);
            }
          }
          return merged;
        });
      }
    };

    try {
      if (!append) {
        browseNextOffsetRef.current = 0;
        browseAppendFailedRef.current = false;
        await withBusy(
          mode === "mods" ? "Searching mods…" : "Searching Nexus…",
          async () => {
            if (mode === "mods") {
              const page = await api.searchCatalog(activeGame.id, query, {
                ...requestOpts,
                sourceFilter,
              });
              if (requestId !== browseRequestIdRef.current) return;
              applyModPage(
                page.items,
                page.total_count,
                page.has_more,
                page.next_offset,
                "replace",
              );
            } else {
              const page = await api.searchCollections(
                activeGame.nexus_domain,
                query,
                requestOpts,
              );
              if (requestId !== browseRequestIdRef.current) return;
              applyCollectionPage(page.items, page.total_count, "replace");
            }
          },
        );
      } else if (mode === "mods") {
        const page = await api.searchCatalog(activeGame.id, query, {
          ...requestOpts,
          sourceFilter,
        });
        if (requestId !== browseRequestIdRef.current) return;
        applyModPage(
          page.items,
          page.total_count,
          page.has_more,
          page.next_offset,
          "append",
        );
        appendOk = true;
        browseAppendFailedRef.current = false;
      } else {
        const page = await api.searchCollections(
          activeGame.nexus_domain,
          query,
          requestOpts,
        );
        if (requestId !== browseRequestIdRef.current) return;
        applyCollectionPage(page.items, page.total_count, "append");
        appendOk = true;
        browseAppendFailedRef.current = false;
      }
    } catch (e) {
      if (requestId === browseRequestIdRef.current) {
        setError(String(e));
        if (!append) {
          browseHasMoreRef.current = false;
          setBrowseHasMore(false);
        } else {
          browseAppendFailedRef.current = true;
        }
      }
    } finally {
      if (append) {
        browseLoadingMoreRef.current = false;
        if (
          appendOk &&
          requestId === browseRequestIdRef.current &&
          browseHasMoreRef.current &&
          isBrowseSentinelNearView()
        ) {
          queueMicrotask(() => {
            void runSearch(
              browseModeRef.current,
              searchQueryRef.current,
              browseOptsRef.current,
              true,
            );
          });
        }
      }
    }
  }

  function isBrowseSentinelNearView(): boolean {
    const sentinel = browseSentinelRef.current;
    const root = mainScrollRef.current;
    if (!sentinel || !root) return false;
    const margin = 240;
    const rootRect = root.getBoundingClientRect();
    const sentRect = sentinel.getBoundingClientRect();
    return (
      sentRect.top < rootRect.bottom + margin &&
      sentRect.bottom > rootRect.top - margin
    );
  }

  async function search() {
    await runSearch();
  }

  function cycleBrowseTag(tag: string) {
    const tags = setTagInMap(browseTags, tag);
    setBrowseTags(tags);
    const { tagsInclude, tagsExclude } = tagListsFromMap(tags);
    runSearch(browseMode, searchQuery, {
      ...browseOpts,
      tagsInclude,
      tagsExclude,
      offset: 0,
    });
  }

  function clearBrowseFilters() {
    setBrowseCategory("");
    setBrowseVersion("");
    setBrowseTags({});
    setTagQuery("");
    setTagsOpen(false);
    runSearch(browseMode, searchQuery, {
      ...browseOpts,
      category: null,
      gameVersion: null,
      tagsInclude: [],
      tagsExclude: [],
      offset: 0,
    });
  }

  async function openMod(hit: CatalogHit, detailTab: DetailTab = "info") {
    if (!activeGame) return;
    setBrowseDetail({ kind: "mod", hit, tab: detailTab });
    setModDetail(null);
    setTsDetail(null);
    setTsVersion("");
    setCollectionModFiles([]);
    setCollectionDetail(null);
    setModioDetail(null);
    setModioFiles([]);
    if (hit.source === "thunderstore") {
      if (!hit.community || !hit.namespace || !hit.package_name) return;
      await withBusy("Loading package…", async () => {
        const detail = await api.getThunderstorePackage(
          hit.community!,
          hit.namespace!,
          hit.package_name!,
        );
        setTsDetail(detail);
        setTsVersion(detail.latest_version ?? detail.versions[0]?.version_number ?? "");
      });
      return;
    }
    if (hit.source === "modio") {
      if (hit.modio_game_id == null || hit.modio_mod_id == null) return;
      await withBusy("Loading mod.io mod…", async () => {
        const detail = await api.getModioMod(hit.modio_game_id!, hit.modio_mod_id!);
        setModioDetail(detail);
        if (detailTab === "files") {
          setModioFiles(
            await api.modioFiles(hit.modio_game_id!, hit.modio_mod_id!),
          );
        }
      });
      return;
    }
    const domain = hit.domain_name || activeGame.nexus_domain;
    const modId = hit.mod_id;
    if (!modId) return;
    if (detailTab === "files") {
      setModFiles([]);
      await withBusy("Loading files…", async () => {
        setModFiles(await api.modFiles(domain, modId));
      });
    } else {
      await withBusy("Loading mod…", async () => {
        setModDetail(await api.getMod(domain, modId));
      });
    }
  }

  async function openCollection(hit: CollectionHit, detailTab: DetailTab = "info") {
    if (!activeGame) return;
    setBrowseDetail({ kind: "collection", hit, tab: detailTab });
    setModDetail(null);
    setModFiles([]);
    setCollectionModFiles([]);
    setCollectionDetail(null);
    const domain = hit.domain_name || activeGame.nexus_domain;
    await withBusy(
      detailTab === "files" ? "Loading collection mods…" : "Loading collection…",
      async () => {
        const detailPromise = api.getCollection({ slug: hit.slug, domain });
        const filesPromise = api.collectionFiles({
          slug: hit.slug,
          revision: hit.revision_number,
        });
        const [detail, files] = await Promise.all([detailPromise, filesPromise]);
        setCollectionDetail(detail);
        setCollectionModFiles(files);
      },
    );
  }

  async function setDetailTab(detailTab: DetailTab) {
    if (!browseDetail || !activeGame) return;
    if (browseDetail.tab === detailTab) return;
    const next = { ...browseDetail, tab: detailTab };
    setBrowseDetail(next);
    if (next.kind === "mod") {
      if (next.hit.source === "thunderstore") {
        if (!tsDetail && next.hit.community && next.hit.namespace && next.hit.package_name) {
          await withBusy("Loading package…", async () => {
            const detail = await api.getThunderstorePackage(
              next.hit.community!,
              next.hit.namespace!,
              next.hit.package_name!,
            );
            setTsDetail(detail);
            setTsVersion(detail.latest_version ?? detail.versions[0]?.version_number ?? "");
          });
        }
      } else if (next.hit.source === "modio") {
        if (
          next.hit.modio_game_id != null &&
          next.hit.modio_mod_id != null &&
          (!modioDetail || (detailTab === "files" && modioFiles.length === 0))
        ) {
          await withBusy(
            detailTab === "files" ? "Loading files…" : "Loading mod.io mod…",
            async () => {
              if (!modioDetail) {
                setModioDetail(
                  await api.getModioMod(
                    next.hit.modio_game_id!,
                    next.hit.modio_mod_id!,
                  ),
                );
              }
              if (detailTab === "files" && modioFiles.length === 0) {
                setModioFiles(
                  await api.modioFiles(
                    next.hit.modio_game_id!,
                    next.hit.modio_mod_id!,
                  ),
                );
              }
            },
          );
        }
      } else if (detailTab === "files" && modFiles.length === 0 && next.hit.mod_id) {
        await withBusy("Loading files…", async () => {
          setModFiles(
            await api.modFiles(
              next.hit.domain_name || activeGame.nexus_domain,
              next.hit.mod_id!,
            ),
          );
        });
      } else if (detailTab === "info" && !modDetail && next.hit.mod_id) {
        await withBusy("Loading mod…", async () => {
          setModDetail(
            await api.getMod(
              next.hit.domain_name || activeGame.nexus_domain,
              next.hit.mod_id!,
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
    } else if (detailTab === "info" && !collectionDetail) {
      await withBusy("Loading collection…", async () => {
        setCollectionDetail(
          await api.getCollection({
            slug: next.hit.slug,
            domain: next.hit.domain_name || activeGame.nexus_domain,
          }),
        );
      });
    }
  }

  function closeBrowseDetail() {
    setBrowseDetail(null);
    setModDetail(null);
    setTsDetail(null);
    setTsVersion("");
    setModioDetail(null);
    setModioFiles([]);
    setModFiles([]);
    setCollectionModFiles([]);
    setCollectionDetail(null);
  }

  async function openGame(game: ManagedGame, detailTab: GameDetailTab = "info") {
    await api.setActiveGame(game.id);
    setActiveId(game.id);
    setLibraryDetail({ game, tab: detailTab });
    setUeDomainEdit(game.nexus_domain);
    setUeProjectEdit(game.project_name ?? "");
    setTsCommunityEdit(game.thunderstore_community ?? "");
    setModioGameIdEdit(
      game.modio_game_id != null && game.modio_game_id > 0
        ? String(game.modio_game_id)
        : "",
    );
    setTab("library");
    setGameInfo(null);
    if (game.nexus_domain) {
      try {
        setGameInfo(await api.getGame(game.nexus_domain));
      } catch {
        /* Nexus metadata is optional for local game info */
      }
    }
  }

  async function installThunderstorePackage() {
    if (!activeGame || browseDetail?.kind !== "mod") return;
    const hit = browseDetail.hit;
    if (hit.source !== "thunderstore" || !hit.community || !hit.namespace || !hit.package_name) {
      return;
    }
    setTab("downloads");
    setBusy(`Installing ${hit.name}…`);
    setError(null);
    try {
      await api.downloadThunderstoreMod({
        gameId: activeGame.id,
        community: hit.community,
        namespace: hit.namespace,
        name: hit.package_name,
        version: tsVersion || null,
      });
      await refreshMods(activeGame.id);
      await refreshDownloads();
      setNotice({ kind: "ok", message: `Installed ${hit.name} (with dependencies).` });
    } catch (e) {
      setError(String(e));
      await refreshDownloads();
    } finally {
      setBusy(null);
    }
  }

  function setGameDetailTab(detailTab: GameDetailTab) {
    setLibraryDetail((prev) => (prev ? { ...prev, tab: detailTab } : prev));
  }

  function closeLibraryDetail() {
    setLibraryDetail(null);
    setGameInfo(null);
  }

  async function installModioFile(fileId?: number | null, version?: string | null) {
    if (!activeGame || browseDetail?.kind !== "mod") return;
    const hit = browseDetail.hit;
    if (hit.source !== "modio" || hit.modio_game_id == null || hit.modio_mod_id == null) {
      return;
    }
    setTab("downloads");
    setBusy(`Installing ${hit.name}…`);
    setError(null);
    try {
      await api.downloadModioMod({
        gameId: activeGame.id,
        modioGameId: hit.modio_game_id,
        modId: hit.modio_mod_id,
        fileId: fileId ?? null,
        name: hit.name,
        version: version ?? null,
        installDeps: true,
      });
      await refreshMods(activeGame.id);
      await refreshDownloads();
      setNotice({ kind: "ok", message: `Installed ${hit.name}.` });
    } catch (e) {
      setError(String(e));
      await refreshDownloads();
    } finally {
      setBusy(null);
    }
  }


  async function installFile(file: ModFileInfo) {
    if (!activeGame || browseDetail?.kind !== "mod") return;
    const selectedMod = browseDetail.hit;
    if (selectedMod.source !== "nexus" || selectedMod.mod_id == null) return;
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
        const msg = String(e);
        if (!msg.includes("Paused")) {
          setError(msg);
        }
        await refreshDownloads();
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
  const downloadStatus = statusSummary(busy, assistQueue, downloads, activeBatch);

  function openDownloadsTab() {
    setTab("downloads");
    void refreshDownloads();
  }

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark">E</span>
          <div>
            <strong>Emperor Mod Manager</strong>
            <small>MVP+</small>
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

      <main className="main" ref={mainScrollRef}>
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
                Clear Nexus API key
              </button>
            )}
            <h2 style={{ marginTop: "1.5rem" }}>mod.io</h2>
            <p>
              Optional read-only API key from{" "}
              <a
                href="https://mod.io/me/access"
                target="_blank"
                rel="noreferrer"
              >
                mod.io → Access
              </a>
              . Used to browse and download mods for games with a mod.io game ID.
              OAuth, uploads, and ratings are not supported.
            </p>
            {settings?.has_modio_api_key && (
              <div className="status-card">
                <span className="pill ok">mod.io API key saved</span>
              </div>
            )}
            <div className="row">
              <input
                type="password"
                placeholder="mod.io API key"
                value={modioKeyInput}
                onChange={(e) => setModioKeyInput(e.target.value)}
              />
              <button onClick={saveModioApiKey} disabled={!modioKeyInput.trim()}>
                Save & validate
              </button>
            </div>
            {settings?.has_modio_api_key && (
              <button
                className="danger"
                onClick={() =>
                  withBusy("Clearing mod.io key…", async () => {
                    await api.clearModioApiKey();
                    await refreshSettings();
                  })
                }
              >
                Clear mod.io API key
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
                      {gameInfo?.genre ? ` · ${gameInfo.genre}` : ""}
                    </p>
                    <div className="detail-stats">
                      <span>
                        {mods.length} mod{mods.length === 1 ? "" : "s"}
                      </span>
                      <span>
                        {mods.filter((m) => m.enabled).length} enabled
                      </span>
                      {gameInfo?.mods != null && (
                        <span>{gameInfo.mods.toLocaleString()} on Nexus</span>
                      )}
                      {gameInfo?.downloads != null && (
                        <span>{gameInfo.downloads.toLocaleString()} downloads</span>
                      )}
                      <span>{libraryDetail.game.nexus_domain}</span>
                    </div>
                    <div className="detail-mod-actions">
                      <button
                        type="button"
                        onClick={() =>
                          openExternal(
                            gameInfo?.nexusmods_url ||
                              `https://www.nexusmods.com/${libraryDetail.game.nexus_domain}`,
                          )
                        }
                      >
                        Open on Nexus
                      </button>
                      {gameInfo?.forum_url && (
                        <button
                          type="button"
                          onClick={() => openExternal(gameInfo.forum_url!)}
                        >
                          Forum
                        </button>
                      )}
                      {libraryDetail.tab === "mods" && (
                        <>
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
                        </>
                      )}
                    </div>
                  </div>
                </div>

                {libraryDetail.tab === "info" ? (
                  <div className="detail-body">
                    <h2>About</h2>
                    <dl className="meta">
                      <dt>Launcher</dt>
                      <dd>{libraryDetail.game.launcher}</dd>
                      <dt>Nexus domain</dt>
                      <dd>{libraryDetail.game.nexus_domain || "—"}</dd>
                      {libraryDetail.game.thunderstore_community && (
                        <>
                          <dt>Thunderstore</dt>
                          <dd>{libraryDetail.game.thunderstore_community}</dd>
                        </>
                      )}
                      {libraryDetail.game.modio_game_id != null &&
                        libraryDetail.game.modio_game_id > 0 && (
                        <>
                          <dt>mod.io game ID</dt>
                          <dd>{libraryDetail.game.modio_game_id}</dd>
                        </>
                      )}
                      <dt>Install</dt>
                      <dd>{libraryDetail.game.install_path}</dd>
                      <dt>Plugin</dt>
                      <dd>{libraryDetail.game.plugin_id}</dd>
                      {libraryDetail.game.project_name && (
                        <>
                          <dt>UE project</dt>
                          <dd>{libraryDetail.game.project_name}</dd>
                        </>
                      )}
                      {gameInfo?.genre && (
                        <>
                          <dt>Genre</dt>
                          <dd>{gameInfo.genre}</dd>
                        </>
                      )}
                      {gameInfo?.mods != null && (
                        <>
                          <dt>Nexus mods</dt>
                          <dd>{gameInfo.mods.toLocaleString()}</dd>
                        </>
                      )}
                      {gameInfo?.file_count != null && (
                        <>
                          <dt>Nexus files</dt>
                          <dd>{gameInfo.file_count.toLocaleString()}</dd>
                        </>
                      )}
                      {gameInfo?.downloads != null && (
                        <>
                          <dt>Downloads</dt>
                          <dd>{gameInfo.downloads.toLocaleString()}</dd>
                        </>
                      )}
                    </dl>
                    <div className="panel" style={{ marginTop: "1rem" }}>
                        <h3>Catalog & deploy settings</h3>
                        <p className="note">
                          Override Nexus domain, Thunderstore community, mod.io game ID,
                          or Unreal project folder when auto-detect is wrong. At least one
                          catalog source is required to browse.
                        </p>
                        <label>
                          Nexus domain
                          <input
                            value={ueDomainEdit}
                            onChange={(e) => setUeDomainEdit(e.target.value)}
                            placeholder="optional if Thunderstore/mod.io only"
                          />
                        </label>
                        <label>
                          Thunderstore community
                          <input
                            value={tsCommunityEdit}
                            onChange={(e) => setTsCommunityEdit(e.target.value)}
                            placeholder="e.g. lethal-company, valheim"
                          />
                        </label>
                        <label>
                          mod.io game ID
                          <input
                            value={modioGameIdEdit}
                            onChange={(e) => setModioGameIdEdit(e.target.value)}
                            placeholder="numeric id from mod.io"
                            inputMode="numeric"
                          />
                        </label>
                        {(libraryDetail.game.plugin_id === "unreal" ||
                          libraryDetail.game.plugin_id === "stalker2heartofchornobyl" ||
                          libraryDetail.game.plugin_id === "palworld" ||
                          libraryDetail.game.plugin_id === "hogwartslegacy") && (
                          <label>
                            Project folder
                            <input
                              value={ueProjectEdit}
                              onChange={(e) => setUeProjectEdit(e.target.value)}
                              placeholder="e.g. Pal, Phoenix, Stalker2"
                            />
                          </label>
                        )}
                        <button type="button" onClick={() => void saveUeGameSettings()}>
                          Save
                        </button>
                      </div>
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
                                {m.source === "thunderstore"
                                  ? `Thunderstore · ${m.ts_namespace ?? "?"}-${m.ts_name ?? "?"}`
                                  : m.source === "modio"
                                    ? `mod.io · game ${m.modio_game_id ?? "?"} · mod ${m.modio_mod_id ?? "?"} · file ${m.modio_file_id ?? "?"}`
                                    : `#${m.nexus_mod_id} · file ${m.nexus_file_id}`}
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
                        <li className="empty">No staged mods. Browse mods to install some.</li>
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
                  Supported plugins include Stardew Valley, Baldur&apos;s Gate 3,
                  Cyberpunk 2077, Days Gone, S.T.A.L.K.E.R. 2, Palworld, Hogwarts
                  Legacy, Darktide, FromSoftware and Resident Evil titles, seeded
                  Unity/BepInEx games (Lethal Company, Valheim, Risk of Rain 2, and
                  more), plus generic Unreal and BepInEx pipelines. Browse can merge
                  Nexus, Thunderstore, and mod.io when each source is configured.
                </p>
                {unrealManage && (
                  <div className="panel unreal-manage-panel">
                    <h2>Add as Unreal Engine game</h2>
                    <p>
                      <strong>{unrealManage.title}</strong> looks like an Unreal
                      install. Catalog IDs are suggested from the game title when
                      possible — edit them if needed, then confirm the project
                      folder.
                    </p>
                    <label>
                      Nexus domain (optional)
                      <input
                        value={unrealDomain}
                        onChange={(e) => setUnrealDomain(e.target.value)}
                        placeholder="e.g. palworld"
                        autoFocus
                      />
                      {catalogHints?.nexus_name ? (
                        <span className="field-hint">
                          Matched: {catalogHints.nexus_name}
                        </span>
                      ) : null}
                    </label>
                    <label>
                      Thunderstore community (optional)
                      <input
                        value={unrealCommunity}
                        onChange={(e) => setUnrealCommunity(e.target.value)}
                        placeholder="e.g. palworld"
                      />
                      {catalogHints?.thunderstore_name ? (
                        <span className="field-hint">
                          Matched: {catalogHints.thunderstore_name}
                        </span>
                      ) : null}
                    </label>
                    <label>
                      mod.io game ID (optional)
                      <input
                        value={unrealModioId}
                        onChange={(e) => setUnrealModioId(e.target.value)}
                        placeholder="e.g. 1234"
                        inputMode="numeric"
                      />
                      {catalogHints?.modio_name ? (
                        <span className="field-hint">
                          Matched: {catalogHints.modio_name}
                        </span>
                      ) : null}
                    </label>
                    <label>
                      Project folder (optional)
                      <input
                        value={unrealProject}
                        onChange={(e) => setUnrealProject(e.target.value)}
                        placeholder="Auto-detected when possible"
                      />
                    </label>
                    <div className="row-actions">
                      <button onClick={() => void confirmUnrealManage()}>
                        Manage
                      </button>
                      <button
                        type="button"
                        onClick={() => {
                          catalogSuggestGen.current += 1;
                          setUnrealManage(null);
                          setCatalogHints(null);
                        }}
                      >
                        Cancel
                      </button>
                    </div>
                  </div>
                )}
                {bepinexManage && (
                  <div className="panel unreal-manage-panel">
                    <h2>Add as Unity / BepInEx game</h2>
                    <p>
                      <strong>{bepinexManage.title}</strong> looks like a Unity
                      install. Catalog IDs are suggested from the game title when
                      possible — edit them if needed.
                    </p>
                    <label>
                      Nexus domain (optional)
                      <input
                        value={bepinexDomain}
                        onChange={(e) => setBepinexDomain(e.target.value)}
                        placeholder="e.g. lethalcompany"
                        autoFocus
                      />
                      {catalogHints?.nexus_name ? (
                        <span className="field-hint">
                          Matched: {catalogHints.nexus_name}
                        </span>
                      ) : null}
                    </label>
                    <label>
                      Thunderstore community (optional)
                      <input
                        value={bepinexCommunity}
                        onChange={(e) => setBepinexCommunity(e.target.value)}
                        placeholder="e.g. lethal-company"
                      />
                      {catalogHints?.thunderstore_name ? (
                        <span className="field-hint">
                          Matched: {catalogHints.thunderstore_name}
                        </span>
                      ) : null}
                    </label>
                    <label>
                      mod.io game ID (optional)
                      <input
                        value={bepinexModioId}
                        onChange={(e) => setBepinexModioId(e.target.value)}
                        placeholder="e.g. 1234"
                        inputMode="numeric"
                      />
                      {catalogHints?.modio_name ? (
                        <span className="field-hint">
                          Matched: {catalogHints.modio_name}
                        </span>
                      ) : null}
                    </label>
                    <div className="row-actions">
                      <button onClick={() => void confirmBepinexManage()}>
                        Manage
                      </button>
                      <button
                        type="button"
                        onClick={() => {
                          catalogSuggestGen.current += 1;
                          setBepinexManage(null);
                          setCatalogHints(null);
                        }}
                      >
                        Cancel
                      </button>
                    </div>
                  </div>
                )}
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
                                {g.launcher} · {g.nexus_domain || g.thunderstore_community || g.plugin_id}
                              </small>
                            </div>
                          </li>
                        ))}
                      </ul>
                    )}
                  </>
                )}
                <h2>Unmanaged</h2>
                {libraryView === "grid" ? (
                  <div className="media-grid">
                    {unmanaged.map((g) =>
                      g.supported ? (
                        <MediaCard
                          key={g.id}
                          title={g.title}
                          imageSrc={mediaSrc(g.cover_path)}
                          hoverLabel="Manage"
                          onClick={() => manage(g)}
                        />
                      ) : g.engine_hint === "unreal" && g.install_path ? (
                        <MediaCard
                          key={g.id}
                          title={g.title}
                          imageSrc={mediaSrc(g.cover_path)}
                          hoverLabel="Add as Unreal"
                          onClick={() => beginUnrealManage(g)}
                        />
                      ) : g.engine_hint === "bepinex" && g.install_path ? (
                        <MediaCard
                          key={g.id}
                          title={g.title}
                          imageSrc={mediaSrc(g.cover_path)}
                          hoverLabel="Add as BepInEx"
                          onClick={() => beginBepinexManage(g)}
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
                    {unmanaged.length === 0 && (
                      <p className="note">
                        No unmanaged games found. Scan again if you installed something
                        while the app was open.
                      </p>
                    )}
                  </div>
                ) : (
                  <ul className="list">
                    {unmanaged.map((g) => (
                      <li key={g.id}>
                        <div>
                          <strong>{g.title}</strong>
                          <small>
                            {g.launcher}
                            {g.install_path ? ` · ${g.install_path}` : ""}
                            {!g.supported &&
                            g.engine_hint !== "unreal" &&
                            g.engine_hint !== "bepinex"
                              ? " · unsupported"
                              : ""}
                            {g.engine_hint === "unreal" && !g.supported
                              ? " · Unreal Engine"
                              : ""}
                            {g.engine_hint === "bepinex" && !g.supported
                              ? " · Unity / BepInEx"
                              : ""}
                          </small>
                        </div>
                        {g.supported ? (
                          <button onClick={() => manage(g)}>Manage</button>
                        ) : g.engine_hint === "unreal" && g.install_path ? (
                          <button onClick={() => beginUnrealManage(g)}>
                            Add as Unreal
                          </button>
                        ) : g.engine_hint === "bepinex" && g.install_path ? (
                          <button onClick={() => beginBepinexManage(g)}>
                            Add as BepInEx
                          </button>
                        ) : null}
                      </li>
                    ))}
                    {unmanaged.length === 0 && (
                      <li className="empty">
                        No unmanaged games found. Scan again if you installed something
                        while the app was open.
                      </li>
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
                    {!(
                      browseDetail.kind === "mod" &&
                      browseDetail.hit.source === "thunderstore"
                    ) && (
                      <button
                        className={browseDetail.tab === "files" ? "active" : ""}
                        onClick={() => setDetailTab("files")}
                      >
                        Files
                      </button>
                    )}
                  </div>
                </div>

                {browseDetail.kind === "mod" ? (
                  <>
                    <div className="detail-hero">
                      <div className="detail-cover">
                        {mediaSrc(
                          null,
                          (browseDetail.hit.source === "modio"
                            ? modioDetail?.picture_url
                            : modDetail?.picture_url) ?? browseDetail.hit.picture_url,
                        ) ? (
                          <img
                            src={
                              mediaSrc(
                                null,
                                (browseDetail.hit.source === "modio"
                                  ? modioDetail?.picture_url
                                  : modDetail?.picture_url) ??
                                  browseDetail.hit.picture_url,
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
                          {(browseDetail.hit.source === "modio"
                            ? modioDetail?.author
                            : modDetail?.author) ??
                            browseDetail.hit.author ??
                            "Unknown author"}
                          {modDetail?.uploaded_by &&
                            browseDetail.hit.source !== "modio" &&
                            modDetail.uploaded_by !==
                              (modDetail.author ?? browseDetail.hit.author) && (
                              <span className="detail-author-secondary">
                                {" "}
                                · uploaded by {modDetail.uploaded_by}
                              </span>
                            )}
                        </p>
                        <div className="detail-stats">
                          {browseDetail.hit.source !== "modio" &&
                            (modDetail?.endorsements ?? browseDetail.hit.endorsements) !=
                              null && (
                            <span>
                              {(
                                modDetail?.endorsements ?? browseDetail.hit.endorsements
                              )!.toLocaleString()}{" "}
                              endorsements
                            </span>
                          )}
                          {(
                            (browseDetail.hit.source === "modio"
                              ? modioDetail?.downloads
                              : modDetail?.downloads) ?? browseDetail.hit.downloads
                          ) != null && (
                            <span>
                              {(
                                (browseDetail.hit.source === "modio"
                                  ? modioDetail?.downloads
                                  : modDetail?.downloads) ?? browseDetail.hit.downloads
                              )!.toLocaleString()}{" "}
                              downloads
                            </span>
                          )}
                          {modDetail?.version && browseDetail.hit.source !== "modio" && (
                            <span>v{modDetail.version}</span>
                          )}
                          {(modDetail?.category ?? browseDetail.hit.category) &&
                            browseDetail.hit.source !== "modio" && (
                            <span>{modDetail?.category ?? browseDetail.hit.category}</span>
                          )}
                          {modDetail?.contains_adult_content && (
                            <span className="detail-badge-adult">Adult</span>
                          )}
                          {modDetail?.status &&
                            modDetail.status.toLowerCase() !== "published" &&
                            modDetail.status.toLowerCase() !== "normal" && (
                              <span className="detail-badge-status">{modDetail.status}</span>
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
                        <div className="detail-mod-actions">
                          {browseDetail.hit.source === "thunderstore" ? (
                            <>
                              <button
                                type="button"
                                onClick={() =>
                                  openExternal(
                                    browseDetail.hit.package_url ||
                                      `https://thunderstore.io/c/${browseDetail.hit.community}/p/${browseDetail.hit.namespace}/${browseDetail.hit.package_name}/`,
                                  )
                                }
                              >
                                Open on Thunderstore
                              </button>
                              <label className="inline-version">
                                Version
                                <select
                                  value={tsVersion}
                                  onChange={(e) => setTsVersion(e.target.value)}
                                >
                                  {(tsDetail?.versions ?? []).map((v) => (
                                    <option key={v.uuid4} value={v.version_number}>
                                      {v.version_number}
                                      {v.downloads
                                        ? ` · ${v.downloads.toLocaleString()} dl`
                                        : ""}
                                    </option>
                                  ))}
                                </select>
                              </label>
                              <button
                                type="button"
                                onClick={() => void installThunderstorePackage()}
                              >
                                Install
                              </button>
                            </>
                          ) : browseDetail.hit.source === "modio" ? (
                            <>
                              <button
                                type="button"
                                onClick={() =>
                                  openExternal(
                                    browseDetail.hit.profile_url ||
                                      modioDetail?.profile_url ||
                                      `https://mod.io/g/${browseDetail.hit.modio_game_id}/m/${browseDetail.hit.modio_mod_id}`,
                                  )
                                }
                              >
                                Open on mod.io
                              </button>
                              <button
                                type="button"
                                onClick={() =>
                                  void installModioFile(
                                    modioDetail?.primary_file_id ?? null,
                                    null,
                                  )
                                }
                              >
                                Install primary
                              </button>
                            </>
                          ) : (
                            <button
                              type="button"
                              onClick={() =>
                                openExternal(
                                  `https://www.nexusmods.com/${
                                    modDetail?.domain_name ||
                                    browseDetail.hit.domain_name ||
                                    activeGame?.nexus_domain
                                  }/mods/${browseDetail.hit.mod_id}`,
                                )
                              }
                            >
                              Open on Nexus
                            </button>
                          )}
                          {modDetail?.uploaded_users_profile_url && (
                            <button
                              type="button"
                              onClick={() =>
                                openExternal(modDetail.uploaded_users_profile_url!)
                              }
                            >
                              Author profile
                            </button>
                          )}
                        </div>
                      </div>
                    </div>

                    {browseDetail.tab === "info" ? (
                      <div className="detail-body">
                        <h2>About</h2>
                        <RichText
                          className="detail-description detail-description-html"
                          text={
                            browseDetail.hit.source === "thunderstore"
                              ? (tsDetail?.description ??
                                browseDetail.hit.summary)
                              : browseDetail.hit.source === "modio"
                                ? (modioDetail?.description ??
                                  modioDetail?.summary ??
                                  browseDetail.hit.summary)
                                : (modDetail?.description ??
                                  modDetail?.summary ??
                                  browseDetail.hit.summary)
                          }
                        />
                        {browseDetail.hit.source === "modio" &&
                          modioDetail?.has_dependencies && (
                          <p className="note">
                            This mod lists dependencies on mod.io; missing ones are
                            installed automatically before the primary file.
                          </p>
                        )}
                        {browseDetail.hit.source === "thunderstore" &&
                          tsDetail?.versions?.[0]?.dependencies?.length ? (
                          <>
                            <h2>Dependencies</h2>
                            <ul className="list">
                              {(
                                tsDetail.versions.find(
                                  (v) => v.version_number === tsVersion,
                                ) ?? tsDetail.versions[0]
                              ).dependencies.map((d) => (
                                <li key={d}>
                                  <strong>{d}</strong>
                                </li>
                              ))}
                            </ul>
                            <p className="note">
                              Missing dependencies are installed automatically with the
                              package.
                            </p>
                          </>
                        ) : null}
                      </div>
                    ) : (
                      <div className="detail-body">
                        <h2>Files</h2>
                        {browseDetail.hit.source === "modio" ? (
                          <ul className="list">
                            {modioFiles.map((f) => (
                              <li key={f.file_id}>
                                <div>
                                  <strong>{f.filename}</strong>
                                  <small>
                                    {f.version ? `v${f.version}` : "file"}
                                    {f.is_primary ? " · primary" : ""}
                                    {f.filesize
                                      ? ` · ${(f.filesize / (1024 * 1024)).toFixed(1)} MB`
                                      : ""}
                                  </small>
                                </div>
                                <button
                                  onClick={() =>
                                    void installModioFile(f.file_id, f.version)
                                  }
                                >
                                  Install
                                </button>
                              </li>
                            ))}
                            {modioFiles.length === 0 && (
                              <li className="empty">No files found for this mod.</li>
                            )}
                          </ul>
                        ) : (
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
                        )}
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
                        <p className="detail-author">
                          {browseDetail.hit.author ?? "Collection"}
                        </p>
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
                          {browseDetail.hit.mod_count != null && (
                            <span>
                              {browseDetail.hit.mod_count.toLocaleString()} mods
                            </span>
                          )}
                          {collectionModFiles.length > 0 && (
                            <span>
                              {collectionModFiles.length.toLocaleString()} files
                            </span>
                          )}
                          {browseDetail.hit.file_size != null &&
                            browseDetail.hit.file_size > 0 && (
                              <span>{formatBytes(browseDetail.hit.file_size)}</span>
                            )}
                          {browseDetail.hit.overall_rating != null && (
                            <span>
                              {browseDetail.hit.overall_rating.toFixed(1)}
                              {browseDetail.hit.overall_rating_count != null
                                ? ` · ${browseDetail.hit.overall_rating_count.toLocaleString()} ratings`
                                : ""}
                            </span>
                          )}
                          {browseDetail.hit.revision_number != null && (
                            <span>rev {browseDetail.hit.revision_number}</span>
                          )}
                          {browseDetail.hit.category && (
                            <span>{browseDetail.hit.category}</span>
                          )}
                          {browseDetail.hit.domain_name && (
                            <span>{browseDetail.hit.domain_name}</span>
                          )}
                        </div>
                        <dl className="meta detail-dates">
                          {formatIsoDate(browseDetail.hit.created_at) && (
                            <>
                              <dt>Created</dt>
                              <dd>{formatIsoDate(browseDetail.hit.created_at)}</dd>
                            </>
                          )}
                          {formatIsoDate(browseDetail.hit.updated_at) && (
                            <>
                              <dt>Updated</dt>
                              <dd>{formatIsoDate(browseDetail.hit.updated_at)}</dd>
                            </>
                          )}
                        </dl>
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
                        <RichText
                          className="detail-description detail-description-html"
                          text={
                            collectionDetail?.description ??
                            collectionDetail?.summary ??
                            browseDetail.hit.summary
                          }
                        />
                      </div>
                    ) : (
                      <div className="detail-body">
                        {(() => {
                          const groups = groupCollectionMods(collectionModFiles);
                          return (
                            <>
                              <h2>
                                Included mods ({groups.length})
                                {collectionModFiles.length > 0 &&
                                collectionModFiles.length !== groups.length
                                  ? ` · ${collectionModFiles.length} files`
                                  : ""}
                              </h2>
                              <ul className="list">
                                {groups.map((group) => {
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
                                            source: "nexus",
                                            id: `nexus:${group.domain_name}:${group.mod_id}`,
                                            name: group.mod_name,
                                            summary: null,
                                            picture_url: null,
                                            downloads: null,
                                            endorsements: null,
                                            author: null,
                                            category: null,
                                            tags: [],
                                            mod_id: group.mod_id,
                                            domain_name: group.domain_name,
                                            community: null,
                                            namespace: null,
                                            package_name: null,
                                            full_name: null,
                                            package_url: null,
                                            rating_score: null,
                                            latest_version: null,
                                            modio_game_id: null,
                                            modio_mod_id: null,
                                            profile_url: null,
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
                            </>
                          );
                        })()}
                      </div>
                    )}
                  </>
                )}
              </>
            ) : (
              <>
                <div className="panel-head">
                  <h1>Browse Mods</h1>
                  <div className="panel-head-actions">
                    <div className="segment">
                      <button
                        className={browseMode === "mods" ? "active" : ""}
                        onClick={() => {
                          setBrowseMode("mods");
                          setBrowseSort("endorsements");
                          setBrowseCategory("");
                          setBrowseVersion("");
                          setBrowseTags({});
                          setBrowseDetail(null);
                          runSearch("mods", searchQuery, {
                            ...browseOpts,
                            sort: "endorsements",
                            category: null,
                            gameVersion: null,
                            tagsInclude: [],
                            tagsExclude: [],
                            offset: 0,
                          });
                        }}
                      >
                        Mods
                      </button>
                      <button
                        className={browseMode === "collections" ? "active" : ""}
                        onClick={() => {
                          setBrowseMode("collections");
                          setBrowseSort("endorsements");
                          setBrowseCategory("");
                          setBrowseVersion("");
                          setBrowseTags({});
                          setBrowseDetail(null);
                          runSearch("collections", searchQuery, {
                            ...browseOpts,
                            sort: "endorsements",
                            category: null,
                            gameVersion: null,
                            tagsInclude: [],
                            tagsExclude: [],
                            offset: 0,
                          });
                        }}
                      >
                        Collections
                      </button>
                    </div>
                    {browseMode === "mods" &&
                      [
                        Boolean(activeGame?.nexus_domain),
                        Boolean(activeGame?.thunderstore_community),
                        Boolean(activeGame?.modio_game_id),
                      ].filter(Boolean).length > 1 && (
                        <div className="segment source-filter">
                          {(
                            [
                              ["all", "All"] as const,
                              ...(activeGame?.nexus_domain
                                ? ([["nexus", "Nexus"]] as const)
                                : []),
                              ...(activeGame?.thunderstore_community
                                ? ([["thunderstore", "Thunderstore"]] as const)
                                : []),
                              ...(activeGame?.modio_game_id
                                ? ([["modio", "mod.io"]] as const)
                                : []),
                            ]
                          ).map(([id, label]) => (
                            <button
                              key={id}
                              type="button"
                              className={sourceFilter === id ? "active" : ""}
                              onClick={() => setSourceFilter(id)}
                            >
                              {label}
                            </button>
                          ))}
                        </div>
                      )}
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
                      {browseMode === "collections" && (
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
                      )}
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
                          {activeTagCount === 0
                            ? "Any"
                            : activeTagCount === 1
                              ? Object.keys(browseTags)[0]
                              : `${activeTagCount} tags`}
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
                            <p className="tag-picker-hint">
                              Click: include → exclude → clear
                            </p>
                            <div className="tag-picker-list">
                              {orderedTags.length === 0 ? (
                                <div className="tag-picker-empty">No tags</div>
                              ) : (
                                orderedTags.map((t) => {
                                  const state = browseTags[t];
                                  return (
                                    <button
                                      key={t}
                                      type="button"
                                      role="option"
                                      aria-selected={Boolean(state)}
                                      className={`tag-picker-item${
                                        state === "include"
                                          ? " include"
                                          : state === "exclude"
                                            ? " exclude"
                                            : ""
                                      }`}
                                      onClick={() => cycleBrowseTag(t)}
                                    >
                                      <span className="tag-picker-state" aria-hidden="true">
                                        {state === "include"
                                          ? "+"
                                          : state === "exclude"
                                            ? "−"
                                            : ""}
                                      </span>
                                      <span>{t}</span>
                                    </button>
                                  );
                                })
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
                              m.source === "thunderstore"
                                ? "Thunderstore"
                                : m.source === "modio"
                                  ? "mod.io"
                                  : "Nexus",
                              ...(m.category ? [m.category] : []),
                              ...(m.tags ?? []).slice(0, 2),
                            ];
                            return (
                              <MediaCard
                                key={m.id}
                                title={m.name}
                                imageSrc={mediaSrc(null, m.picture_url)}
                                badge={m.author ?? "Mod"}
                                overlay={
                                  m.source === "nexus" && m.endorsements != null
                                    ? `${m.endorsements.toLocaleString()} endorsements`
                                    : m.downloads != null
                                      ? `${m.downloads.toLocaleString()} downloads`
                                      : null
                                }
                                tags={tags}
                                onClick={() => openMod(m, "info")}
                                actions={
                                  m.source === "thunderstore" ? (
                                    <button
                                      onClick={(e) => {
                                        e.stopPropagation();
                                        openMod(m, "info");
                                      }}
                                    >
                                      Install
                                    </button>
                                  ) : (
                                    <button
                                      onClick={(e) => {
                                        e.stopPropagation();
                                        openMod(m, "files");
                                      }}
                                    >
                                      Files
                                    </button>
                                  )
                                }
                              />
                            );
                          })}
                          {modHits.length === 0 && (
                            <p className="note">
                              {browseSearched
                                ? "No results for this search."
                                : "Search to find mods for this game."}
                            </p>
                          )}
                        </div>
                      ) : (
                        <ul className="list">
                          {modHits.map((m) => (
                            <li key={m.id} className="list-row-clickable">
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
                                  {m.source === "thunderstore"
                                    ? "Thunderstore"
                                    : m.source === "modio"
                                      ? "mod.io"
                                      : "Nexus"}
                                  {" · "}
                                  {m.author ?? "unknown"}
                                  {m.source === "nexus" && m.endorsements != null
                                    ? ` · ${m.endorsements} endorsements`
                                    : m.downloads != null
                                      ? ` · ${m.downloads.toLocaleString()} downloads`
                                      : ""}
                                </small>
                                {m.summary && <p className="summary">{m.summary}</p>}
                              </div>
                              <button
                                onClick={() =>
                                  openMod(
                                    m,
                                    m.source === "thunderstore" ? "info" : "files",
                                  )
                                }
                              >
                                {m.source === "thunderstore" ? "Install" : "Files"}
                              </button>
                            </li>
                          ))}
                        </ul>
                      )
                    ) : browseView === "grid" ? (
                      <div className="media-grid">
                        {collectionHits.map((c) => {
                          const tags = [
                            ...(c.mod_count != null
                              ? [`${c.mod_count.toLocaleString()} mods`]
                              : []),
                            ...(c.file_size != null && c.file_size > 0
                              ? [formatBytes(c.file_size)]
                              : []),
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
                          <p className="note">
                            {browseSearched
                              ? "No results for this search."
                              : "Search to find collections for this game."}
                          </p>
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
                              <small>
                                {c.author ?? c.slug}
                                {c.mod_count != null
                                  ? ` · ${c.mod_count.toLocaleString()} mods`
                                  : ""}
                                {c.file_size != null && c.file_size > 0
                                  ? ` · ${formatBytes(c.file_size)}`
                                  : ""}
                              </small>
                              {c.summary && <p className="summary">{c.summary}</p>}
                            </div>
                            <button onClick={() => installCollection(c)}>Install</button>
                          </li>
                        ))}
                      </ul>
                    )}
                    <div
                      ref={browseSentinelRef}
                      className="browse-scroll-sentinel"
                      aria-hidden="true"
                    />
                    {(modHits.length > 0 || collectionHits.length > 0) && (
                      <p className="browse-page-status note">
                        {browseMode === "mods"
                          ? `Showing ${modHits.length}${
                              browseTotalCount > 0 ? ` of ${browseTotalCount}` : ""
                            } mods`
                          : `Showing ${collectionHits.length}${
                              browseTotalCount > 0
                                ? ` of ${browseTotalCount}`
                                : ""
                            } collections`}
                        {browseHasMore ? " · scroll for more" : ""}
                      </p>
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
            onPauseDownload={(id) => pauseOneDownload(id)}
            onResumeDownload={(id) => resumeOneDownload(id)}
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
