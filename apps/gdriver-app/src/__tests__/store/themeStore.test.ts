import { describe, it, expect, beforeEach, vi } from "vitest";
import { useThemeStore, applyTheme, applyThemeFromMode } from "@/store/themeStore";
import type { ThemeMode } from "@/store/themeStore";

beforeEach(() => {
  useThemeStore.setState({ mode: "follow_system", resolved: "light" });
  document.documentElement.classList.remove("dark");
});

// ── Initial state ──────────────────────────────────────────────────────────

describe("initial state", () => {
  it("defaults to follow_system", () => {
    expect(useThemeStore.getState().mode).toBe("follow_system");
  });

  it("defaults resolved to light", () => {
    expect(useThemeStore.getState().resolved).toBe("light");
  });
});

// ── setMode ────────────────────────────────────────────────────────────────

describe("setMode", () => {
  it.each(["light", "dark", "follow_system"] as ThemeMode[])(
    "sets mode to %s",
    (mode) => {
      useThemeStore.getState().setMode(mode);
      expect(useThemeStore.getState().mode).toBe(mode);
    },
  );
});

// ── setResolved ────────────────────────────────────────────────────────────

describe("setResolved", () => {
  it("sets resolved to dark", () => {
    useThemeStore.getState().setResolved("dark");
    expect(useThemeStore.getState().resolved).toBe("dark");
  });

  it("sets resolved to light", () => {
    useThemeStore.getState().setResolved("dark");
    useThemeStore.getState().setResolved("light");
    expect(useThemeStore.getState().resolved).toBe("light");
  });
});

// ── applyTheme ─────────────────────────────────────────────────────────────

describe("applyTheme", () => {
  it("adds dark class when dark", () => {
    applyTheme("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("removes dark class when light", () => {
    document.documentElement.classList.add("dark");
    applyTheme("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("updates store resolved value", () => {
    applyTheme("dark");
    expect(useThemeStore.getState().resolved).toBe("dark");
    applyTheme("light");
    expect(useThemeStore.getState().resolved).toBe("light");
  });

  it("removes dark class even if not present", () => {
    applyTheme("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });
});

// ── applyThemeFromMode ─────────────────────────────────────────────────────

describe("applyThemeFromMode", () => {
  it("sets store mode", () => {
    applyThemeFromMode("dark");
    expect(useThemeStore.getState().mode).toBe("dark");
  });

  it("applies light theme directly", () => {
    applyThemeFromMode("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    expect(useThemeStore.getState().resolved).toBe("light");
  });

  it("applies dark theme directly", () => {
    applyThemeFromMode("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(useThemeStore.getState().resolved).toBe("dark");
  });

  describe("follow_system", () => {
    let matchMediaMock: ReturnType<typeof vi.fn>;

    beforeEach(() => {
      matchMediaMock = vi.fn();
      window.matchMedia = matchMediaMock as unknown as typeof window.matchMedia;
    });

    it("applies dark when system prefers dark", () => {
      matchMediaMock.mockReturnValue({ matches: true });
      applyThemeFromMode("follow_system");
      expect(document.documentElement.classList.contains("dark")).toBe(true);
      expect(useThemeStore.getState().resolved).toBe("dark");
    });

    it("applies light when system prefers light", () => {
      matchMediaMock.mockReturnValue({ matches: false });
      applyThemeFromMode("follow_system");
      expect(document.documentElement.classList.contains("dark")).toBe(false);
      expect(useThemeStore.getState().resolved).toBe("light");
    });

    it("calls matchMedia with dark scheme query", () => {
      matchMediaMock.mockReturnValue({ matches: false });
      applyThemeFromMode("follow_system");
      expect(matchMediaMock).toHaveBeenCalledWith(
        "(prefers-color-scheme: dark)",
      );
    });
  });
});
