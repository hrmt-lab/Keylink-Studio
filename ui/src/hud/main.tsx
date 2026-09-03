import React from "react";
import ReactDOM from "react-dom/client";
import { listen } from "@tauri-apps/api/event";
import Hud from "./Hud";
import "@fontsource/zen-kaku-gothic-new/400.css";
import "@fontsource/zen-kaku-gothic-new/500.css";
import "@fontsource/spline-sans-mono/400.css";
import "@fontsource/spline-sans-mono/500.css";
import "../index.css";
import "./hud.css";
import { ACCENT_CHANGED_EVENT, applyAccent, initAccent } from "../lib/theme";
import { LangProvider } from "../i18n";

initAccent();

// Applies the accent live when Settings changes it, so the HUD doesn't need
// an app restart to pick up a new color (`lib/theme.ts`'s `setAccent`
// emits this; the HUD window lives for the app's lifetime, so this listener
// is never torn down).
void listen<string>(ACCENT_CHANGED_EVENT, (event) => applyAccent(event.payload));

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <LangProvider>
      <Hud />
    </LangProvider>
  </React.StrictMode>
);
