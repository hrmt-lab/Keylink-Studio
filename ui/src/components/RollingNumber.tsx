import { useRollingNumber } from "../hooks/useRollingNumber";

/** Numeric text that rolls to its new value (use with a monospace font). */
export function RollingNumber({
  value,
  format,
  fallback = "--",
}: {
  value: number | null;
  format: (value: number) => string;
  fallback?: string;
}) {
  const shown = useRollingNumber(value);
  return <>{shown === null ? fallback : format(shown)}</>;
}
