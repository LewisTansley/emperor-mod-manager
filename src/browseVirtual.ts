/** Column gap on `.media-grid`. */
export const BROWSE_GRID_COLUMN_GAP_REM = 0.85;
/** Row gap on `.media-grid`. */
export const BROWSE_GRID_ROW_GAP_REM = 1;
/** Top margin on `.media-grid`. */
export const BROWSE_GRID_PAD_START_REM = 0.5;
/** Bottom margin on `.media-grid`. */
export const BROWSE_GRID_PAD_END_REM = 1;
/** Row gap on `.list`. */
export const BROWSE_LIST_GAP_REM = 0.45;

/**
 * Column count for `repeat(auto-fill, minmax(minPx, 1fr))`.
 * At least one column, even when the container is narrower than `minPx`.
 */
export function gridColumnCount(width: number, minPx: number, gapPx: number): number {
  if (!(width > 0) || !(minPx > 0)) return 1;
  const gap = gapPx > 0 ? gapPx : 0;
  return Math.max(1, Math.floor((width + gap) / (minPx + gap)));
}

/** Width of one track once `gridColumnCount` has chosen how many fit. */
export function gridCardWidth(width: number, columns: number, gapPx: number): number {
  if (!(width > 0) || columns <= 1) return Math.max(width, 0);
  const gap = gapPx > 0 ? gapPx : 0;
  return Math.max(0, (width - gap * (columns - 1)) / columns);
}

/**
 * Row height for a browse card: 2/3 cover, two title lines, and one meta line.
 * Measured rows replace this when a title is only one line.
 */
export function estimateGridRowHeight(cardWidth: number, rootPx: number): number {
  const rem = rootPx > 0 ? rootPx : 16;
  if (!(cardWidth > 0)) return Math.round(16 * rem);
  const cover = cardWidth * 1.5;
  const title = 2 * 0.95 * 1.25 * rem;
  const meta = 0.78 * 1.3 * rem;
  const gaps = 2 * 0.45 * rem;
  const metaPull = 0.15 * rem;
  return Math.round(cover + title + meta + gaps - metaPull);
}

/** Starting list-row height before measurement. Posters reserve the 7.5rem cover. */
export function estimateListRowHeight(posters: boolean, rootPx = 16): number {
  const rem = rootPx > 0 ? rootPx : 16;
  return Math.round((posters ? 14 : 7) * rem);
}

/** Used before the results pane has a size. Matches the catalog default. */
export const BROWSE_PAGE_FALLBACK = 24;
/** Keeps a very tall window from asking for an unbounded page. */
const BROWSE_PAGE_CAP = 120;

/**
 * Hits that fill the scrollport, plus one row past the fold.
 * Returns {@link BROWSE_PAGE_FALLBACK} until the pane has been measured.
 */
export function browsePageCount(
  columns: number,
  viewportHeight: number,
  rowHeight: number,
  rowGap: number,
): number {
  if (!(columns > 0) || !(viewportHeight > 0) || !(rowHeight > 0)) {
    return BROWSE_PAGE_FALLBACK;
  }
  const gap = rowGap > 0 ? rowGap : 0;
  const stride = rowHeight + gap;
  const visibleRows = Math.max(1, Math.ceil((viewportHeight + gap) / stride));
  return Math.min(BROWSE_PAGE_CAP, columns * (visibleRows + 1));
}
