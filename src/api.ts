import { invoke } from "@tauri-apps/api/core";
import type {
  BrowseMeta,
  BrowseSearchOpts,
  CatalogSearchPage,
  CatalogSuggestion,
  CollectionDetail,
  CollectionModFile,
  CollectionSearchPage,
  DeployResult,
  DetectedGame,
  DownloadItem,
  GameInfo,
  ManagedGame,
  ModDetail,
  ModFileInfo,
  ModioFileInfo,
  ModioModDetail,
  ModSearchPage,
  NexusUser,
  Settings,
  StagedMod,
  ThemePreference,
  TsPackageDetail,
  UeLayoutInfo,
} from "./types";

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  setApiKey: (key: string) => invoke<NexusUser>("set_api_key", { key }),
  validateUser: () => invoke<NexusUser>("validate_user"),
  clearApiKey: () => invoke<void>("clear_api_key"),
  setModioApiKey: (key: string) => invoke<void>("set_modio_api_key", { key }),
  clearModioApiKey: () => invoke<void>("clear_modio_api_key"),
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
    projectName?: string | null;
    thunderstoreCommunity?: string | null;
    modioGameId?: number | null;
  }) => invoke<ManagedGame>("manage_game", game),
  updateManagedGame: (args: {
    id: string;
    nexusDomain?: string | null;
    projectName?: string | null;
    thunderstoreCommunity?: string | null;
    modioGameId?: number | null;
  }) => invoke<ManagedGame>("update_managed_game", args),
  detectUeLayout: (installPath: string, projectName?: string | null) =>
    invoke<UeLayoutInfo | null>("detect_ue_layout", {
      installPath,
      projectName: projectName ?? null,
    }),
  suggestCatalogIds: (title: string) =>
    invoke<CatalogSuggestion>("suggest_catalog_ids", { title }),
  unmanageGame: (id: string) => invoke<void>("unmanage_game", { id }),
  setActiveGame: (id: string) => invoke<void>("set_active_game", { id }),
  searchMods: (domain: string, query: string, opts: BrowseSearchOpts = {}) =>
    invoke<ModSearchPage>("search_mods", {
      domain,
      query,
      sort: opts.sort ?? null,
      category: opts.category ?? null,
      tagsInclude: opts.tagsInclude ?? null,
      tagsExclude: opts.tagsExclude ?? null,
      gameVersion: opts.gameVersion ?? null,
      offset: opts.offset ?? null,
      count: opts.count ?? null,
    }),
  searchCatalog: (
    gameId: string,
    query: string,
    opts: BrowseSearchOpts & { sourceFilter?: string | null } = {},
  ) =>
    invoke<CatalogSearchPage>("search_catalog", {
      gameId,
      query,
      sourceFilter: opts.sourceFilter ?? null,
      sort: opts.sort ?? null,
      category: opts.category ?? null,
      tagsInclude: opts.tagsInclude ?? null,
      tagsExclude: opts.tagsExclude ?? null,
      gameVersion: opts.gameVersion ?? null,
      offset: opts.offset ?? null,
      count: opts.count ?? null,
    }),
  getThunderstorePackage: (
    community: string,
    namespace: string,
    name: string,
  ) =>
    invoke<TsPackageDetail>("get_thunderstore_package", {
      community,
      namespace,
      name,
    }),
  downloadThunderstoreMod: (args: {
    gameId: string;
    community: string;
    namespace: string;
    name: string;
    version?: string | null;
  }) => invoke<StagedMod[]>("download_thunderstore_mod", args),
  getModioMod: (gameId: number, modId: number) =>
    invoke<ModioModDetail>("get_modio_mod", { gameId, modId }),
  modioFiles: (gameId: number, modId: number) =>
    invoke<ModioFileInfo[]>("modio_files", { gameId, modId }),
  downloadModioMod: (args: {
    gameId: string;
    modioGameId: number;
    modId: number;
    fileId?: number | null;
    name: string;
    version?: string | null;
    installDeps?: boolean | null;
  }) => invoke<StagedMod[]>("download_modio_mod", args),
  getMod: (domain: string, modId: number) =>
    invoke<ModDetail>("get_mod", { domain, modId }),
  getGame: (domain: string) => invoke<GameInfo>("get_game", { domain }),
  modFiles: (domain: string, modId: number) =>
    invoke<ModFileInfo[]>("mod_files", { domain, modId }),
  searchCollections: (
    domain: string,
    query: string,
    opts: BrowseSearchOpts = {},
  ) =>
    invoke<CollectionSearchPage>("search_collections", {
      domain,
      query,
      sort: opts.sort ?? null,
      category: opts.category ?? null,
      tagsInclude: opts.tagsInclude ?? null,
      tagsExclude: opts.tagsExclude ?? null,
      gameVersion: opts.gameVersion ?? null,
      offset: opts.offset ?? null,
      count: opts.count ?? null,
    }),
  browseMeta: (domain: string) => invoke<BrowseMeta>("browse_meta", { domain }),
  getCollection: (args: { slug: string; domain?: string | null }) =>
    invoke<CollectionDetail>("get_collection", {
      slug: args.slug,
      domain: args.domain ?? null,
    }),
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
  setAssistBounds: (bounds: {
    x: number;
    y: number;
    width: number;
    height: number;
  }) => invoke<void>("set_assist_bounds", bounds),
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
  pauseDownload: (id: string) => invoke<void>("pause_download", { id }),
  resumeDownload: (id: string) => invoke<void>("resume_download", { id }),
};
