import { useEffect, useRef, useState } from "react";

const DURATION_MS = 400;

/**
 * Animates numeric value changes with an ease-out cubic roll (400ms).
 * Returns the value as-is when motion is reduced or there is no previous value.
 */
export function useRollingNumber(value: number | null): number | null {
  const [display, setDisplay] = useState(value);
  const displayRef = useRef(value);

  useEffect(() => {
    const from = displayRef.current;
    const reduced =
      typeof window.matchMedia === "function" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (value === null || from === null || from === value || reduced) {
      displayRef.current = value;
      setDisplay(value);
      return;
    }
    const start = performance.now();
    let raf = 0;
    const tick = (now: number) => {
      const p = Math.min(1, (now - start) / DURATION_MS);
      const eased = 1 - Math.pow(1 - p, 3);
      const next = from + (value - from) * eased;
      displayRef.current = next;
      setDisplay(next);
      if (p < 1) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [value]);

  return display;
}
