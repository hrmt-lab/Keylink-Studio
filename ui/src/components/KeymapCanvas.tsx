import { useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import type { StudioPhysicalKey } from "../types";

interface KeymapCanvasProps {
  keys: StudioPhysicalKey[];
  /** Inner content of each key (label, count, ...). */
  keyContent: (key: StudioPhysicalKey) => ReactNode;
  /** Extra inline style per key (e.g. heatmap background). */
  keyStyle?: (key: StudioPhysicalKey) => CSSProperties | undefined;
  /** Tooltip per key. */
  keyTitle?: (key: StudioPhysicalKey) => string | undefined;
  /** Optional click handler used by keymap editing. */
  onKeyClick?: (key: StudioPhysicalKey, element: HTMLDivElement) => void;
  /** Optional content rendered inside the same plate frame, below the key grid (e.g. encoders). */
  footer?: ReactNode;
}

/**
 * Renders ZMK Studio physical-layout keys at their x/y positions.
 * Shared between the keymap view and the typing-stats heatmap.
 */
export function KeymapCanvas({ keys, keyContent, keyStyle, keyTitle, onKeyClick, footer }: KeymapCanvasProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [containerWidth, setContainerWidth] = useState(0);
  const metrics = useMemo(() => layoutMetrics(keys, containerWidth), [keys, containerWidth]);

  useEffect(() => {
    const element = containerRef.current;
    if (!element) return;

    const updateWidth = () => {
      const style = window.getComputedStyle(element);
      const paddingX =
        Number.parseFloat(style.paddingLeft || "0") +
        Number.parseFloat(style.paddingRight || "0");
      setContainerWidth(Math.max(0, element.clientWidth - paddingX));
    };
    updateWidth();

    const observer = new ResizeObserver(updateWidth);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  // ZMK physical layouts pack keys edge-to-edge; inset each cap so
  // neighbouring keys read as separate keys on the plate.
  const keyGap = 5;

  return (
    <div ref={containerRef} className="max-w-full overflow-hidden rounded-card bg-plate shadow-neu-down p-4">
      <div
        className="relative mx-auto flex-shrink-0"
        style={{ width: metrics.width, height: metrics.height }}
      >
        {keys.map((key) => {
          const x = (key.x - metrics.minX) * metrics.scale + metrics.padding + keyGap / 2;
          const y = (key.y - metrics.minY) * metrics.scale + metrics.padding + keyGap / 2;
          const width = Math.max(16, Math.abs(key.width) * metrics.scale - keyGap);
          const height = Math.max(16, Math.abs(key.height) * metrics.scale - keyGap);
          const originX = (key.rx - key.x) * metrics.scale;
          const originY = (key.ry - key.y) * metrics.scale;
          return (
            <div
              key={`${key.position}-${key.x}-${key.y}`}
              title={keyTitle?.(key)}
              onClick={(event) => onKeyClick?.(key, event.currentTarget)}
              className={`absolute flex flex-col items-center justify-center rounded-lg bg-surface px-1.5 text-center ${onKeyClick ? "cursor-pointer hover:ring-2 hover:ring-accent" : ""}`}
              style={{
                left: x,
                top: y,
                width,
                height,
                // ZMK physical-layout rotation is in 1/100 of a degree.
                transform: key.r ? `rotate(${key.r / 100}deg)` : undefined,
                transformOrigin: `${originX}px ${originY}px`,
                ...keyStyle?.(key),
              }}
            >
              {keyContent(key)}
            </div>
          );
        })}
      </div>
      {footer}
    </div>
  );
}

export function layoutMetrics(keys: StudioPhysicalKey[], availableWidth = 0) {
  const padding = 24;
  const rawMinX = Math.min(...keys.map((key) => key.x));
  const rawMinY = Math.min(...keys.map((key) => key.y));
  const rawMaxX = Math.max(...keys.map((key) => key.x + Math.abs(key.width)));
  const rawMaxY = Math.max(...keys.map((key) => key.y + Math.abs(key.height)));
  const rawWidth = Math.max(1, rawMaxX - rawMinX);
  const rawHeight = Math.max(1, rawMaxY - rawMinY);
  const maxWidth = availableWidth > 0 ? Math.max(220, availableWidth) : 820;
  const scale = Math.min(1.2, Math.max(0.2, (maxWidth - padding * 2) / rawWidth));
  return {
    minX: rawMinX,
    minY: rawMinY,
    scale,
    padding,
    width: rawWidth * scale + padding * 2,
    height: rawHeight * scale + padding * 2,
  };
}
