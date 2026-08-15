export type ThemePreference = "light" | "dark" | "system";

export type DetectedGame = {
  id: string;
  title: string;
  install_path: string | null;
  launcher: string;
  supported: boolean;
  plugin_id: string | null;
  nexus_domain: string | null;
  cover_path: string | null;
};

export type ManagedGame = {
  id: string;
  title: string;
  nexus_domain: string;
  install_path: string;
  launcher: string;
  plugin_id: string;
  cover_path?: string | null;
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
  category?: string | null;
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
  tags: string[];
  game_versions: string[];
};

export type BrowseSearchOpts = {
  sort?: string;
  category?: string | null;
  tags?: string[];
  gameVersion?: string | null;
};

export type StagedMod = {
  id: string;
  name: string;
  nexus_mod_id: number;
  nexus_file_id: number;
  version: string | null;
  domain: string;
  staging_path: string;
  enabled: boolean;
  order: number;
};

export type DeployResult = {
  file_count: number;
  enabled_mods: number;
  warnings: string[];
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
  has_api_key: boolean;
  user: NexusUser | null;
  config_dir: string;
  data_dir: string;
  cache_dir: string;
};
