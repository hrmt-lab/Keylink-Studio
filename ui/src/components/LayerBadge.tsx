import { useEffect, useRef, useState } from "react";

interface Props {
  layer: number | null;
  size?: "sm" | "md" | "lg";
}

const SIZE_CLASSES = {
  sm: "px-2 py-0.5 text-xs",
  md: "px-2.5 py-1 text-sm",
  lg: "px-4 py-1.5 text-lg",
};

export function LayerBadge({ layer, size = "md" }: Props) {
  // Swap with a 140ms fade-out → replace → fade-in.
  const [shown, setShown] = useState(layer);
  const [fading, setFading] = useState(false);
  const timer = useRef<number | null>(null);
  useEffect(() => {
    if (layer === shown) return;
    setFading(true);
    timer.current = window.setTimeout(() => {
      setShown(layer);
      setFading(false);
    }, 140);
    return () => {
      if (timer.current !== null) clearTimeout(timer.current);
    };
  }, [layer, shown]);

  return (
    <span
      className={`badge-swap inline-flex items-center rounded font-mono font-medium ${
        SIZE_CLASSES[size]
      } ${
        shown === null ? "bg-plate text-disabled" : "bg-accent-soft text-accent"
      } ${fading ? "opacity-0" : "opacity-100"}`}
    >
      {shown === null ? "--" : `L${shown}`}
    </span>
  );
}
