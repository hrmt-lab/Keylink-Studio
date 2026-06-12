import { useMemo, type CSSProperties, type ReactNode } from "react";
import type { StudioPhysicalKey } from "../types";

interface KeymapCanvasProps {
  keys: StudioPhysicalKey[];
  /** Inner content of each key (label, count, ...). */
  keyContent: (key: StudioPhysicalKey) => ReactNode;
  /** Extra inline style per key (e.g. heatmap background). */
  keyStyle?: (key: StudioPhysicalKey) => CSSProperties | undefined;
  /** Tooltip per key. */
  keyTitle?: (key: StudioPhysicalKey) => string | undefined;
}

/**
 * Renders ZMK Studio physical-layout keys at their x/y positions.
 * Shared between the keymap view and the typing-stats heatmap.
 */
export function KeymapCanvas({ keys, keyContent, keyStyle, keyTitle }: KeymapCanvasProps) {
  const metrics = useMemo(() => layoutMetrics(keys), [keys]);

  return (
    <div className="max-w-full overflow-x-auto overflow-y-hidden rounded-xl bg-background p-4 ring-1 ring-border">
      <div
        className="relative flex-shrink-0"
        style={{ width: metrics.width, height: metrics.height }}
      >
        {keys.map((key) => {
          const x = (key.x - metrics.minX) * metrics.scale + metrics.padding;
          const y = (key.y - metrics.minY) * metrics.scale + metrics.padding;
          const width = Math.max(16, Math.abs(key.width) * metrics.scale);
          const height = Math.max(16, Math.abs(key.height) * metrics.scale);
          const originX = (key.rx - key.x) * metrics.scale;
          const originY = (key.ry - key.y) * metrics.scale;
          return (
            <div
              key={`${key.position}-${key.x}-${key.y}`}
              title={keyTitle?.(key)}
              className="absolute flex flex-col items-center justify-center rounded-lg border border-border bg-white px-1.5 text-center shadow-sm ring-1 ring-white/70"
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
    </div>
  );
}

export function layoutMetrics(keys: StudioPhysicalKey[]) {
  const padding = 24;
  const rawMinX = Math.min(...keys.map((key) => key.x));
  const rawMinY = Math.min(...keys.map((key) => key.y));
  const rawMaxX = Math.max(...keys.map((key) => key.x + Math.abs(key.width)));
  const rawMaxY = Math.max(...keys.map((key) => key.y + Math.abs(key.height)));
  const rawWidth = Math.max(1, rawMaxX - rawMinX);
  const rawHeight = Math.max(1, rawMaxY - rawMinY);
  const maxWidth = 820;
  const scale = Math.min(1.2, maxWidth / rawWidth);
  return {
    minX: rawMinX,
    minY: rawMinY,
    scale,
    padding,
    width: rawWidth * scale + padding * 2,
    height: rawHeight * scale + padding * 2,
  };
}
