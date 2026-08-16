export type ThemePreference = "light" | "dark" | "system";
export type InstallClickBehavior = "stay" | "downloads";

export type DetectedGame = {
  id: string;
  title: string;
  install_path: string | null;
  launcher: string;
  supported: boolean;
  plugin_id: string | null;
  nexus_domain: string | null;
  cover_path: string | null;
  engine_hint: string | null;
};

export type ManagedGame = {
  id: string;
  title: string;
  nexus_domain: string;
  install_path: string;
  launcher: string;
  plugin_id: string;
  cover_path?: string | null;
  project_name?: string | null;
  thunderstore_community?: string | null;
  modio_game_id?: number | null;
};

export type UeLayoutInfo = {
  project_name: string;
  binaries_platform: string;
  paks_dir: string;
  binaries_dir: string;
};

export type CatalogSuggestion = {
  nexus_domain: string | null;
  nexus_name: string | null;
  thunderstore_community: string | null;
  thunderstore_name: string | null;
  modio_game_id: number | null;
  modio_name: string | null;
};

export type CatalogSuggestRequest = {
  id: string;
  title: string;
};

export type CatalogSource = "nexus" | "thunderstore" | "modio";

export type CatalogHit = {
  source: CatalogSource;
  id: string;
  name: string;
  summary: string | null;
  picture_url: string | null;
  author: string | null;
  downloads: number | null;
  endorsements: number | null;
  category: string | null;
  tags: string[];
  mod_id: number | null;
  domain_name: string | null;
  community: string | null;
  namespace: string | null;
  package_name: string | null;
  full_name: string | null;
  package_url: string | null;
  rating_score: number | null;
  latest_version: string | null;
  modio_game_id: number | null;
  modio_mod_id: number | null;
  profile_url: string | null;
};

export type CatalogSearchPage = {
  items: CatalogHit[];
  total_count: number;
  next_offset: number;
  has_more: boolean;
  nexus_available: boolean;
  thunderstore_available: boolean;
  modio_available: boolean;
};

export type TsPackageVersion = {
  name: string;
  full_name: string;
  description: string;
  icon: string;
  version_number: string;
  dependencies: string[];
  download_url: string;
  downloads: number;
  date_created: string;
  website_url: string;
  is_active: boolean;
  uuid4: string;
  file_size: number;
};

export type TsPackageDetail = {
  community: string;
  namespace: string;
  name: string;
  full_name: string;
  package_url: string;
  uuid4: string;
  rating_score: number;
  is_deprecated: boolean;
  has_nsfw_content: boolean;
  categories: string[];
  description: string | null;
  icon_url: string | null;
  downloads: number;
  versions: TsPackageVersion[];
  latest_version: string | null;
};

export type GameCategory = {
  category_id: number;
  name: string;
};

export type GameInfo = {
  id: number;
  name: string;
  domain_name: string;
  genre: string | null;
  forum_url: string | null;
  nexusmods_url: string | null;
  mods: number | null;
  file_count: number | null;
  downloads: number | null;
  categories: GameCategory[];
};

export type NexusUser = {
  user_id: number;
  key: string;
  name: string;
  is_premium: boolean;
  is_supporter: boolean;
};

export type ModSearchHit = {
  mod_id: number;
  name: string;
  summary: string | null;
  picture_url: string | null;
  downloads: number | null;
  endorsements: number | null;
  author: string | null;
  domain_name: string;
  category?: string | null;
  tags?: string[];
};

export type ModDetail = {
  mod_id: number;
  name: string;
  summary: string | null;
  description: string | null;
  picture_url: string | null;
  author: string | null;
  version: string | null;
  downloads: number | null;
  endorsements: number | null;
  created_timestamp: number | null;
  updated_timestamp: number | null;
  domain_name: string;
  category_id?: number | null;
  category?: string | null;
  uploaded_by?: string | null;
  status?: string | null;
  contains_adult_content?: boolean;
  uploaded_users_profile_url?: string | null;
};

export type ModFileInfo = {
  file_id: number;
  name: string;
  version: string | null;
  category_name: string | null;
  size_kb: number | null;
  uploaded_timestamp: number | null;
  is_primary: boolean;
};

export type CollectionHit = {
  slug: string;
  name: string;
  summary: string | null;
  endorsements: number | null;
  total_downloads: number | null;
  domain_name: string | null;
  revision_number: number | null;
  tile_image_url?: string | null;
  author: string | null;
  category: string | null;
  overall_rating: number | null;
  overall_rating_count: number | null;
  created_at: string | null;
  updated_at: string | null;
  mod_count: number | null;
  file_size: number | null;
};

export type CollectionDetail = {
  slug: string;
  name: string;
  summary: string | null;
  description: string | null;
  endorsements: number | null;
  total_downloads: number | null;
  domain_name: string | null;
  revision_number: number | null;
  tile_image_url?: string | null;
};

export type CollectionModFile = {
  file_id: number;
  optional: boolean;
  mod_id: number;
  mod_name: string;
  file_name: string;
  version: string | null;
  domain_name: string;
};

export type BrowseMeta = {
  categories: string[];
  mod_tags: string[];
  collection_tags: string[];
  game_versions: string[];
};

export type TagFilterState = "include" | "exclude";

export type BrowseSearchOpts = {
  sort?: string;
  category?: string | null;
  tagsInclude?: string[];
  tagsExclude?: string[];
  gameVersion?: string | null;
  offset?: number;
  count?: number;
};

export type ModSearchPage = {
  items: ModSearchHit[];
  nodes_count: number;
  total_count: number;
};

export type CollectionSearchPage = {
  items: CollectionHit[];
  nodes_count: number;
  total_count: number;
};

export type StagedMod = {
  id: string;
  name: string;
  source?: "nexus" | "thunderstore" | "modio";
  nexus_mod_id: number;
  nexus_file_id: number;
  version: string | null;
  domain: string;
  staging_path: string;
  enabled: boolean;
  order: number;
  ts_namespace?: string | null;
  ts_name?: string | null;
  ts_package_uuid?: string | null;
  modio_game_id?: number | null;
  modio_mod_id?: number | null;
  modio_file_id?: number | null;
};

export type ModioModDetail = {
  game_id: number;
  mod_id: number;
  name: string;
  name_id: string;
  summary: string;
  description: string | null;
  picture_url: string | null;
  author: string | null;
  downloads: number;
  profile_url: string;
  tags: string[];
  has_dependencies: boolean;
  primary_file_id: number | null;
};

export type ModioFileInfo = {
  file_id: number;
  filename: string;
  version: string | null;
  filesize: number;
  changelog: string | null;
  is_primary: boolean;
  date_added: number;
};

export type DeployResult = {
  file_count: number;
  enabled_mods: number;
  warnings: string[];
};

export type RecoveryReport = {
  games_copied: string[];
  mods_rewritten: number;
  warnings: string[];
};

export type OrphanScan = {
  legacy_game_ids: string[];
  missing_staging: string[];
  untracked_staging: string[];
  deploy_without_loadorder: string[];
};

export type DownloadItem = {
  id: string;
  label: string;
  status: string;
  error: string | null;
  bytes_downloaded: number;
  bytes_total: number | null;
  speed_bps: number;
  batch_id: string | null;
};

export type Settings = {
  adult_content: boolean;
  autoclick_free_download: boolean;
  last_active_game_id: string | null;
  theme?: ThemePreference;
  install_click_behavior?: InstallClickBehavior;
  has_api_key: boolean;
  has_modio_api_key?: boolean;
  user: NexusUser | null;
  config_dir: string;
  data_dir: string;
  cache_dir: string;
};
