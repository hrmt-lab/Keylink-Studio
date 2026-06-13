/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        accent:        "rgb(var(--accent-rgb) / <alpha-value>)",
        "accent-deep": "rgb(var(--accent-deep-rgb) / <alpha-value>)",
        "accent-soft": "rgb(var(--accent-soft-rgb) / <alpha-value>)",
        background:    "#F1F3F6",
        surface:       "#FFFFFF",
        border:        "#E6E9ED",
        plate:         "#EAEEF3",
        ink:           "#21242A",
        muted:         "#8C9097",
        faint:         "#A5AAB2",
        disabled:      "#C2C8D1",
        gauge:         "#8C95A3",
      },
      borderRadius: { card: "16px", pill: "10px" },
      fontFamily: {
        sans: ['"Zen Kaku Gothic New"', '"Yu Gothic UI"', "Segoe UI", "sans-serif"],
        mono: ['"Spline Sans Mono"', "Consolas", "monospace"],
      },
      boxShadow: {
        "neu-up":        "4px 4px 9px #CDD4DD, -4px -4px 9px #FFFFFF",
        "neu-down":      "inset 4px 4px 9px #CDD4DD, inset -4px -4px 9px #FFFFFF",
        "neu-sel":       "3px 3px 7px #CDD4DD, -3px -3px 7px #FFFFFF",
        "neu-sel-in":    "inset 2.5px 2.5px 6px #B9C2CE, inset -2.5px -2.5px 6px #FFFFFF",
        "neu-groove":    "inset 1.5px 1.5px 3px #CDD4DD, inset -1.5px -1.5px 3px #FFFFFF",
        "neu-toggle-on": "inset 2px 2px 4px rgb(var(--accent-shade-rgb)), inset -1px -1px 3px rgb(var(--accent-tint-rgb))",
        "neu-knob":      "1px 1px 3px rgba(60,70,90,0.35)",
      },
    },
  },
  plugins: [],
};
