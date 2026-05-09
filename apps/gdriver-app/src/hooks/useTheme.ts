import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  useThemeStore,
  applyTheme,
  applyThemeFromMode,
  type ThemeMode,
} from "@/store/themeStore";

/**
 * Initialize theme from saved preferences and listen for system theme changes.
 * Call once from the app root — works across both onboarding and main phases.
 */
export function useTheme() {
  useEffect(() => {
    // 1. Load saved appearance preference and apply it.
    invoke<{ general: { appearance: ThemeMode } }>("get_preferences")
      .then((prefs) => {
        if (prefs?.general?.appearance) {
          applyThemeFromMode(prefs.general.appearance);
        } else {
          applyThemeFromMode("follow_system");
        }
      })
      .catch(() => {
        // Preferences not available yet (e.g. daemon not connected).
        applyThemeFromMode("follow_system");
      });

    // 2. Listen for system theme changes via matchMedia (primary mechanism).
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onMediaChange = (e: MediaQueryListEvent) => {
      if (useThemeStore.getState().mode === "follow_system") {
        applyTheme(e.matches ? "dark" : "light");
      }
    };
    mq.addEventListener("change", onMediaChange);

    // 3. Listen for Rust-emitted theme event (backup / initial detection).
    const unlistenPromise = listen<string>(
      "system:theme-changed",
      ({ payload }) => {
        if (useThemeStore.getState().mode === "follow_system") {
          applyTheme(payload === "dark" ? "dark" : "light");
        }
      },
    );

    return () => {
      mq.removeEventListener("change", onMediaChange);
      unlistenPromise.then((fn) => fn());
    };
  }, []);
}
