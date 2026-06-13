/**
 * Accent-color theming. The accent and its derived tones live in CSS variables
 * (RGB triplets, see index.css :root) so the whole Tailwind palette follows a
 * single user-picked base color. UI-only preference, persisted in localStorage.
 */

export const DEFAULT_ACCENT = "#EF5B25";

export const PRESET_ACCENTS = [
  "#EF5B25", // orange (default)
  "#14A089", // teal
  "#378ADD", // blue
  "#7F77DD", // purple
  "#D4537E", // pink
  "#D5A021", // gold
];

const ACCENT_KEY = "ui.accentColor";
const CUSTOM_KEY = "ui.accentCustomColors";
const MAX_CUSTOM = 8;

interface Rgb { r: number; g: number; b: number }

function hexToRgb(hex: string): Rgb | null {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const v = parseInt(m[1], 16);
  return { r: (v >> 16) & 0xff, g: (v >> 8) & 0xff, b: v & 0xff };
}

/** Linear mix toward a target channel value (0 = keep, 1 = target). */
function mix(c: Rgb, target: number, t: number): Rgb {
  return {
    r: Math.round(c.r + (target - c.r) * t),
    g: Math.round(c.g + (target - c.g) * t),
    b: Math.round(c.b + (target - c.b) * t),
  };
}

function triplet(c: Rgb): string {
  return `${c.r} ${c.g} ${c.b}`;
}

/** Set the accent CSS variables from a base color. Derived tones follow the
 *  ratios of the original palette (EF5B25 → D54E1E / FAECE7 / B23E14 / FF8A5C). */
export function applyAccent(hex: string): void {
  const base = hexToRgb(hex) ?? hexToRgb(DEFAULT_ACCENT)!;
  const style = document.documentElement.style;
  style.setProperty("--accent-rgb", triplet(base));
  style.setProperty("--accent-deep-rgb", triplet(mix(base, 0, 0.13)));   // text on light
  style.setProperty("--accent-soft-rgb", triplet(mix(base, 255, 0.88))); // pale fill
  style.setProperty("--accent-shade-rgb", triplet(mix(base, 0, 0.3)));   // toggle inner shadow
  style.setProperty("--accent-tint-rgb", triplet(mix(base, 255, 0.32))); // toggle inner highlight
}

function normalize(hex: string): string {
  return `#${hex.replace("#", "").toUpperCase()}`;
}

export function getAccent(): string {
  const stored = localStorage.getItem(ACCENT_KEY);
  return stored && hexToRgb(stored) ? normalize(stored) : DEFAULT_ACCENT;
}

export function setAccent(hex: string): void {
  if (!hexToRgb(hex)) return;
  localStorage.setItem(ACCENT_KEY, normalize(hex));
  applyAccent(hex);
}

/** Apply the persisted accent on startup. */
export function initAccent(): void {
  applyAccent(getAccent());
}

export function getCustomAccents(): string[] {
  try {
    const raw = JSON.parse(localStorage.getItem(CUSTOM_KEY) ?? "[]");
    if (!Array.isArray(raw)) return [];
    return raw.filter((c): c is string => typeof c === "string" && hexToRgb(c) !== null).map(normalize);
  } catch {
    return [];
  }
}

export function addCustomAccent(hex: string): string[] {
  if (!hexToRgb(hex)) return getCustomAccents();
  const color = normalize(hex);
  const list = getCustomAccents().filter((c) => c !== color);
  list.push(color);
  const trimmed = list.slice(-MAX_CUSTOM);
  localStorage.setItem(CUSTOM_KEY, JSON.stringify(trimmed));
  return trimmed;
}

export function removeCustomAccent(hex: string): string[] {
  const color = normalize(hex);
  const list = getCustomAccents().filter((c) => c !== color);
  localStorage.setItem(CUSTOM_KEY, JSON.stringify(list));
  return list;
}
