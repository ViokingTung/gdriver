import { create } from "zustand";

export type ThemeMode = "light" | "dark" | "follow_system";
export type ResolvedTheme = "light" | "dark";

interface ThemeState {
  mode: ThemeMode;
  resolved: ResolvedTheme;
  setMode: (mode: ThemeMode) => void;
  setResolved: (theme: ResolvedTheme) => void;
}

export const useThemeStore = create<ThemeState>((set) => ({
  mode: "follow_system",
  resolved: "light",
  setMode: (mode) => set({ mode }),
  setResolved: (resolved) => set({ resolved }),
}));

/** Apply the resolved theme to the document root element. */
export function applyTheme(resolved: ResolvedTheme) {
  const root = document.documentElement;
  if (resolved === "dark") {
    root.classList.add("dark");
  } else {
    root.classList.remove("dark");
  }
  useThemeStore.getState().setResolved(resolved);
}

/** Resolve a ThemeMode to a concrete theme and apply it. */
export function applyThemeFromMode(mode: ThemeMode) {
  useThemeStore.getState().setMode(mode);

  if (mode === "follow_system") {
    const prefersDark = window.matchMedia(
      "(prefers-color-scheme: dark)",
    ).matches;
    applyTheme(prefersDark ? "dark" : "light");
  } else {
    applyTheme(mode);
  }
}
