import type { CatalogHit, CollectionHit } from "./types";

export type AppTab =
  | "setup"
  | "library"
  | "collections"
  | "browse"
  | "downloads"
  | "tools"
  | "settings";

export type DetailTab = "info" | "files";

export type GameDetailTab = "info" | "mods" | "collections" | "tools";

export type BrowseDetailRef =
  | { kind: "mod"; hit: CatalogHit; tab: DetailTab }
  | { kind: "collection"; hit: CollectionHit; tab: DetailTab };

export type NavEntry = {
  tab: AppTab;
  browseReturnTab: AppTab | null;
  browseDetail: BrowseDetailRef | null;
  libraryDetail: { gameId: string; tab: GameDetailTab } | null;
  gameSavedDetailId: string | null;
  savedDetailId: string | null;
};

export class AppNavHistory {
  private entries: NavEntry[];
  private index: number;

  constructor(initial: NavEntry) {
    this.entries = [initial];
    this.index = 0;
  }

  get length(): number {
    return this.entries.length;
  }

  get currentIndex(): number {
    return this.index;
  }

  current(): NavEntry {
    return this.entries[this.index];
  }

  replaceCurrent(entry: NavEntry) {
    this.entries[this.index] = entry;
  }

  push(entry: NavEntry) {
    this.entries = this.entries.slice(0, this.index + 1);
    this.entries.push(entry);
    this.index = this.entries.length - 1;
  }

  back(): NavEntry | null {
    if (this.index <= 0) return null;
    this.index -= 1;
    return this.entries[this.index];
  }

  forward(): NavEntry | null {
    if (this.index >= this.entries.length - 1) return null;
    this.index += 1;
    return this.entries[this.index];
  }

  reset(entry: NavEntry) {
    this.entries = [entry];
    this.index = 0;
  }
}
