import { useCallback, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  BROWSE_GRID_COLUMN_GAP_REM,
  BROWSE_GRID_PAD_END_REM,
  BROWSE_GRID_PAD_START_REM,
  BROWSE_GRID_ROW_GAP_REM,
  BROWSE_LIST_GAP_REM,
  browsePageCount,
  estimateGridRowHeight,
  gridCardWidth,
  gridColumnCount,
} from "./browseVirtual";

const OVERSCAN = 2;

type ScrollRef = RefObject<HTMLDivElement | null>;

function readRootRem(): number {
  if (typeof document === "undefined") return 16;
  const next = Number.parseFloat(getComputedStyle(document.documentElement).fontSize);
  return next > 0 ? next : 16;
}

function useRootRem(): number {
  const [rem, setRem] = useState(readRootRem);
  useLayoutEffect(() => {
    const sync = () => {
      const next = readRootRem();
      setRem((prev) => (prev === next ? prev : next));
    };
    sync();
    window.addEventListener("resize", sync);
    return () => window.removeEventListener("resize", sync);
  }, []);
  return rem;
}

function useScrollWidth(scrollRef: ScrollRef): number {
  const [width, setWidth] = useState(() => scrollRef.current?.clientWidth ?? 0);
  useLayoutEffect(() => {
    let frame = 0;
    let observer: ResizeObserver | null = null;
    let stopped = false;

    const publish = (next: number) => {
      if (next > 0) setWidth((prev) => (prev === next ? prev : next));
    };

    const watch = () => {
      if (stopped) return;
      const el = scrollRef.current;
      if (!el) {
        frame = requestAnimationFrame(watch);
        return;
      }
      if (!observer) {
        observer = new ResizeObserver(() => publish(el.clientWidth));
        observer.observe(el);
      }
      const next = el.clientWidth;
      publish(next);
      if (next === 0) frame = requestAnimationFrame(watch);
    };

    watch();
    return () => {
      stopped = true;
      cancelAnimationFrame(frame);
      observer?.disconnect();
    };
  }, [scrollRef]);
  return width;
}

type BrowseVirtualGridProps<T> = {
  items: readonly T[];
  scrollRef: ScrollRef;
  minCardPx: number;
  getKey: (item: T) => string;
  renderItem: (item: T, index: number) => ReactNode;
  onPageSize?: (count: number) => void;
};

export function BrowseVirtualGrid<T,>({
  items,
  scrollRef,
  minCardPx,
  getKey,
  renderItem,
  onPageSize,
}: BrowseVirtualGridProps<T>) {
  const width = useScrollWidth(scrollRef);
  const rem = useRootRem();
  const columnGap = BROWSE_GRID_COLUMN_GAP_REM * rem;
  const columns = gridColumnCount(width, minCardPx, columnGap);
  const cardWidth = gridCardWidth(width, columns, columnGap);
  const rowGap = BROWSE_GRID_ROW_GAP_REM * rem;
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el || width <= 0) return;
    const report = () => {
      onPageSize?.(
        browsePageCount(
          columns,
          el.clientHeight,
          estimateGridRowHeight(cardWidth, rem),
          rowGap,
        ),
      );
    };
    report();
    const observer = new ResizeObserver(report);
    observer.observe(el);
    return () => observer.disconnect();
  }, [cardWidth, columns, onPageSize, rem, rowGap, scrollRef, width]);
  // Width 0 is one full-bleed column. Wait until the pane has been measured.
  const rowCount = width > 0 ? Math.ceil(items.length / columns) : 0;
  const getKeyRef = useRef(getKey);
  getKeyRef.current = getKey;
  const getRowKey = useCallback(
    (row: number) => {
      const item = items[row * columns];
      const key = item ? getKeyRef.current(item) : String(row);
      return `${width}:${columns}:${key}`;
    },
    [columns, items, width],
  );

  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => estimateGridRowHeight(cardWidth, rem),
    overscan: OVERSCAN,
    gap: rowGap,
    paddingStart: BROWSE_GRID_PAD_START_REM * rem,
    paddingEnd: BROWSE_GRID_PAD_END_REM * rem,
    initialOffset: () => scrollRef.current?.scrollTop ?? 0,
    getItemKey: getRowKey,
  });

  return (
    <div
      className="browse-virtual"
      style={{ height: virtualizer.getTotalSize() }}
    >
      {virtualizer.getVirtualItems().map((row) => {
        const start = row.index * columns;
        const slice = items.slice(start, start + columns);
        return (
          <div
            key={row.key}
            data-index={row.index}
            ref={virtualizer.measureElement}
            className="browse-virtual-row browse-virtual-grid-row"
            style={{
              transform: `translateY(${row.start}px)`,
              gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
              columnGap,
            }}
          >
            {slice.map((item, offset) => (
              <div key={getKey(item)} className="browse-virtual-cell">
                {renderItem(item, start + offset)}
              </div>
            ))}
          </div>
        );
      })}
    </div>
  );
}

type BrowseVirtualListProps<T> = {
  items: readonly T[];
  scrollRef: ScrollRef;
  getKey: (item: T) => string;
  renderItem: (item: T, index: number) => ReactNode;
  rowClassName: string;
  estimateSize: number;
  onPageSize?: (count: number) => void;
};

export function BrowseVirtualList<T,>({
  items,
  scrollRef,
  getKey,
  renderItem,
  rowClassName,
  estimateSize,
  onPageSize,
}: BrowseVirtualListProps<T>) {
  const width = useScrollWidth(scrollRef);
  const rem = useRootRem();
  const rowGap = BROWSE_LIST_GAP_REM * rem;
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const report = () => {
      onPageSize?.(browsePageCount(1, el.clientHeight, estimateSize, rowGap));
    };
    report();
    const observer = new ResizeObserver(report);
    observer.observe(el);
    return () => observer.disconnect();
  }, [estimateSize, onPageSize, rowGap, scrollRef]);
  const getKeyRef = useRef(getKey);
  getKeyRef.current = getKey;
  const getRowKey = useCallback(
    (index: number) => {
      const item = items[index];
      const key = item ? getKeyRef.current(item) : String(index);
      return `${width}:${estimateSize}:${key}`;
    },
    [estimateSize, items, width],
  );

  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => estimateSize,
    overscan: OVERSCAN,
    gap: rowGap,
    initialOffset: () => scrollRef.current?.scrollTop ?? 0,
    getItemKey: getRowKey,
  });

  return (
    <ul className="list browse-virtual" style={{ height: virtualizer.getTotalSize() }}>
      {virtualizer.getVirtualItems().map((row) => {
        const item = items[row.index];
        if (!item) return null;
        return (
          <li
            key={row.key}
            data-index={row.index}
            ref={virtualizer.measureElement}
            className={`browse-virtual-row ${rowClassName}`}
            style={{ transform: `translateY(${row.start}px)` }}
          >
            {renderItem(item, row.index)}
          </li>
        );
      })}
    </ul>
  );
}
