import { invoke } from "@tauri-apps/api/core";
import type {
  BrowseMeta,
  BrowseSearchOpts,
  CollectionHit,
  CollectionModFile,
  DeployResult,
  DetectedGame,
  DownloadItem,
  ManagedGame,
  ModDetail,
  ModFileInfo,
  ModSearchHit,
  NexusUser,
  Settings,
  StagedMod,
  ThemePreference,
} from "./types";

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  setApiKey: (key: string) => invoke<NexusUser>("set_api_key", { key }),
  validateUser: () => invoke<NexusUser>("validate_user"),
  clearApiKey: () => invoke<void>("clear_api_key"),
  setAdultContent: (enabled: boolean) =>
    invoke<void>("set_adult_content", { enabled }),
  setAutoclickFreeDownload: (enabled: boolean) =>
    invoke<void>("set_autoclick_free_download", { enabled }),
  setTheme: (theme: ThemePreference) => invoke<void>("set_theme", { theme }),
  scanGames: () => invoke<DetectedGame[]>("scan_games"),
  listManaged: () => invoke<ManagedGame[]>("list_managed"),
  manageGame: (game: {
    id: string;
    title: string;
    nexusDomain: string;
    installPath: string;
    launcher: string;
    pluginId: string;
    coverPath?: string | null;
  }) => invoke<ManagedGame>("manage_game", game),
  unmanageGame: (id: string) => invoke<void>("unmanage_game", { id }),
  setActiveGame: (id: string) => invoke<void>("set_active_game", { id }),
  searchMods: (domain: string, query: string, opts: BrowseSearchOpts = {}) =>
    invoke<ModSearchHit[]>("search_mods", {
      domain,
      query,
      sort: opts.sort ?? null,
      category: opts.category ?? null,
      tags: opts.tags ?? null,
      gameVersion: opts.gameVersion ?? null,
    }),
  getMod: (domain: string, modId: number) =>
    invoke<ModDetail>("get_mod", { domain, modId }),
  modFiles: (domain: string, modId: number) =>
    invoke<ModFileInfo[]>("mod_files", { domain, modId }),
  searchCollections: (
    domain: string,
    query: string,
    opts: BrowseSearchOpts = {},
  ) =>
    invoke<CollectionHit[]>("search_collections", {
      domain,
      query,
      sort: opts.sort ?? null,
      category: opts.category ?? null,
      tags: opts.tags ?? null,
      gameVersion: opts.gameVersion ?? null,
    }),
  browseMeta: (domain: string) => invoke<BrowseMeta>("browse_meta", { domain }),
  downloadMod: (args: {
    gameId: string;
    domain: string;
    modId: number;
    fileId: number;
    name: string;
    version?: string | null;
  }) => invoke<StagedMod>("download_mod", args),
  openDownloadAssist: (args: {
    gameId: string;
    domain: string;
    modId: number;
    fileId: number;
    name: string;
    batchId?: string | null;
  }) =>
    invoke<void>("open_download_assist", {
      gameId: args.gameId,
      domain: args.domain,
      modId: args.modId,
      fileId: args.fileId,
      name: args.name,
      batchId: args.batchId ?? null,
    }),
  setAssistBounds: (bounds: { x: number; y: number; width: number; height: number }) =>
    invoke<void>("set_assist_bounds", bounds),
  setAssistVisible: (visible: boolean) =>
    invoke<void>("set_assist_visible", { visible }),
  closeDownloadAssist: () => invoke<void>("close_download_assist"),
  clearAssistSession: () => invoke<void>("clear_assist_session"),
  collectionFiles: (args: { slug: string; revision?: number | null }) =>
    invoke<CollectionModFile[]>("collection_files", {
      slug: args.slug,
      revision: args.revision ?? null,
    }),
  installCollection: (args: {
    gameId: string;
    slug: string;
    revision?: number | null;
    includeOptional: boolean;
  }) => invoke<number>("install_collection", args),
  handleNxm: (url: string) => invoke<DownloadItem>("handle_nxm", { url }),
  importModArchive: (args: {
    gameId: string;
    path: string;
    name?: string | null;
    domain?: string | null;
    modId?: number | null;
    fileId?: number | null;
  }) => invoke<StagedMod>("import_mod_archive", args),
  importAssistDownload: (path: string) =>
    invoke<StagedMod>("import_assist_download", { path }),
  listMods: (gameId: string) => invoke<StagedMod[]>("list_mods", { gameId }),
  setModEnabled: (gameId: string, modId: string, enabled: boolean) =>
    invoke<void>("set_mod_enabled", { gameId, modId, enabled }),
  setLoadOrder: (gameId: string, orderedIds: string[]) =>
    invoke<void>("set_load_order", { gameId, orderedIds }),
  removeMod: (gameId: string, modId: string) =>
    invoke<void>("remove_mod", { gameId, modId }),
  removeAllMods: (gameId: string) =>
    invoke<void>("remove_all_mods", { gameId }),
  deployMods: (gameId: string) => invoke<DeployResult>("deploy_mods", { gameId }),
  purgeMods: (gameId: string) => invoke<void>("purge_mods", { gameId }),
  listDownloads: () => invoke<DownloadItem[]>("list_downloads"),
  cancelDownload: (id: string) => invoke<void>("cancel_download", { id }),
  cancelDownloadBatch: (batchId: string) =>
    invoke<void>("cancel_download_batch", { batchId }),
};
