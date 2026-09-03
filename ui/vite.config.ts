import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Two Vite entries: the main app window (index.html) and the HUD overlay
// window (hud.html, see ui/src/hud/). The dev server serves both without
// extra config (any root-level .html file is servable as-is); this
// `rollupOptions.input` is what makes `vite build` emit both bundles instead
// of just index.html. See docs/ai-approval-hud-design.md §7.3 and
// docs/hud-focus-gate-results.md §7-3 for why the HUD must be a bundled
// route (so it can `import "@tauri-apps/api"`) rather than a bare static
// HTML file under ui/public/.
export default defineConfig(async () => ({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL("./index.html", import.meta.url)),
        hud: fileURLToPath(new URL("./hud.html", import.meta.url)),
      },
    },
  },
}));
