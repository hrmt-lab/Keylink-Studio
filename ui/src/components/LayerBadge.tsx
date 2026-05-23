interface Props {
  layer: number | null;
  size?: "sm" | "md" | "lg";
}

const LAYER_COLORS = [
  "bg-blue-100 text-blue-700 ring-blue-200",
  "bg-violet-100 text-violet-700 ring-violet-200",
  "bg-emerald-100 text-emerald-700 ring-emerald-200",
  "bg-amber-100 text-amber-700 ring-amber-200",
  "bg-rose-100 text-rose-700 ring-rose-200",
  "bg-cyan-100 text-cyan-700 ring-cyan-200",
  "bg-orange-100 text-orange-700 ring-orange-200",
  "bg-pink-100 text-pink-700 ring-pink-200",
];

export function LayerBadge({ layer, size = "md" }: Props) {
  if (layer === null) {
    return (
      <span
        className={`inline-flex items-center rounded-full font-mono font-medium ring-1 ${
          size === "sm" ? "px-2 py-0.5 text-xs" :
          size === "lg" ? "px-4 py-1.5 text-base" :
          "px-2.5 py-1 text-sm"
        } bg-gray-100 text-gray-400 ring-gray-200`}
      >
        --
      </span>
    );
  }

  const colorClass = LAYER_COLORS[layer % LAYER_COLORS.length];
  return (
    <span
      className={`inline-flex items-center rounded-full font-mono font-semibold ring-1 ${
        size === "sm" ? "px-2 py-0.5 text-xs" :
        size === "lg" ? "px-4 py-1.5 text-lg" :
        "px-2.5 py-1 text-sm"
      } ${colorClass}`}
    >
      L{layer}
    </span>
  );
}
