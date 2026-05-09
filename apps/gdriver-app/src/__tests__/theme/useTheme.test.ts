import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { useThemeStore } from "@/store/themeStore";

// ── Mocks must be set up before any import that triggers them ──────────────

const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => mockListen(...args),
}));

import { useTheme } from "@/hooks/useTheme";
import { renderHook, act } from "@testing-library/react";

// ── Helpers ────────────────────────────────────────────────────────────────

function mockMatchMedia(initialMatches = false) {
  let handler: ((e: { matches: boolean }) => void) | null = null;
  const addEventListener = vi.fn((_event: string, h: typeof handler) => {
    handler = h;
  });
  const removeEventListener = vi.fn();
  const mql = {
    matches: initialMatches,
    addEventListener,
    removeEventListener,
  };
  window.matchMedia = vi.fn().mockReturnValue(mql);
  return {
    mql,
    fireChange: (matches: boolean) => {
      handler?.({ matches });
    },
  };
}

beforeEach(() => {
  useThemeStore.setState({ mode: "follow_system", resolved: "light" });
  document.documentElement.classList.remove("dark");
  mockInvoke.mockReset();
  mockListen.mockReset();
  // Default: system prefers light
  window.matchMedia = vi.fn().mockReturnValue({
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  });
});

afterEach(() => {
  vi.clearAllMocks();
});

// ── Initialization ─────────────────────────────────────────────────────────

describe("useTheme initialization", () => {
  it("applies theme from saved preferences", async () => {
    mockInvoke.mockResolvedValueOnce({
      general: { appearance: "dark" },
    });
    mockListen.mockResolvedValueOnce(vi.fn());

    renderHook(() => useTheme());

    await vi.waitFor(() => {
      expect(useThemeStore.getState().mode).toBe("dark");
    });
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("falls back to follow_system when preferences unavailable", async () => {
    mockInvoke.mockRejectedValueOnce(new Error("daemon not connected"));
    mockListen.mockResolvedValueOnce(vi.fn());

    renderHook(() => useTheme());

    await vi.waitFor(() => {
      expect(useThemeStore.getState().mode).toBe("follow_system");
    });
  });

  it("falls back to follow_system when appearance field missing", async () => {
    mockInvoke.mockResolvedValueOnce({ general: {} });
    mockListen.mockResolvedValueOnce(vi.fn());

    renderHook(() => useTheme());

    await vi.waitFor(() => {
      expect(useThemeStore.getState().mode).toBe("follow_system");
    });
  });

  it("registers matchMedia listener on mount", async () => {
    const { mql } = mockMatchMedia(false);
    mockInvoke.mockResolvedValueOnce({ general: {} });
    mockListen.mockResolvedValueOnce(vi.fn());

    const { unmount } = renderHook(() => useTheme());

    await vi.waitFor(() => {
      expect(window.matchMedia).toHaveBeenCalledWith(
        "(prefers-color-scheme: dark)",
      );
    });
    expect(mql.addEventListener).toHaveBeenCalledWith(
      "change",
      expect.any(Function),
    );

    unmount();
    expect(mql.removeEventListener).toHaveBeenCalledWith(
      "change",
      expect.any(Function),
    );
  });

  it("registers Tauri event listener on mount and cleans up", async () => {
    const unlisten = vi.fn();
    mockInvoke.mockResolvedValueOnce({ general: {} });
    mockListen.mockResolvedValueOnce(unlisten);

    const { unmount } = renderHook(() => useTheme());

    await vi.waitFor(() => {
      expect(mockListen).toHaveBeenCalledWith(
        "system:theme-changed",
        expect.any(Function),
      );
    });

    unmount();
    await vi.waitFor(() => {
      expect(unlisten).toHaveBeenCalled();
    });
  });
});

// ── Media query change ─────────────────────────────────────────────────────

describe("media query change", () => {
  it("reacts to system dark mode change when mode is follow_system", async () => {
    const { fireChange } = mockMatchMedia(false);
    mockInvoke.mockResolvedValueOnce({ general: {} });
    mockListen.mockResolvedValueOnce(vi.fn());

    renderHook(() => useTheme());

    await vi.waitFor(() => {
      expect(useThemeStore.getState().mode).toBe("follow_system");
    });

    act(() => fireChange(true));
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(useThemeStore.getState().resolved).toBe("dark");

    act(() => fireChange(false));
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    expect(useThemeStore.getState().resolved).toBe("light");
  });

  it("ignores system theme change when mode is explicit", async () => {
    const { fireChange } = mockMatchMedia(false);
    mockInvoke.mockResolvedValueOnce({
      general: { appearance: "light" },
    });
    mockListen.mockResolvedValueOnce(vi.fn());

    renderHook(() => useTheme());

    await vi.waitFor(() => {
      expect(useThemeStore.getState().mode).toBe("light");
    });

    act(() => fireChange(true));
    // Should stay light — mode is explicitly "light"
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });
});

// ── Tauri event handling ───────────────────────────────────────────────────

describe("Tauri theme event", () => {
  it("reacts to system:theme-changed when mode is follow_system", async () => {
    let tauriHandler: ((event: { payload: string }) => void) | undefined;
    mockListen.mockImplementation(
      (_event: string, handler: (event: { payload: string }) => void) => {
        tauriHandler = handler;
        return Promise.resolve(vi.fn());
      },
    );
    // Reject invoke so it falls back to follow_system via .catch
    mockInvoke.mockRejectedValueOnce(new Error("no prefs"));

    renderHook(() => useTheme());

    // Wait for the invoke rejection + applyThemeFromMode("follow_system")
    await vi.waitFor(() => {
      expect(mockInvoke).toHaveBeenCalled();
    });
    // Give a tick for microtasks
    await vi.waitFor(() => {
      expect(useThemeStore.getState().mode).toBe("follow_system");
    });

    act(() => {
      tauriHandler!({ payload: "dark" });
    });

    await vi.waitFor(() => {
      expect(document.documentElement.classList.contains("dark")).toBe(true);
    });
  });

  it("ignores system:theme-changed when mode is explicit", async () => {
    let tauriHandler: ((event: { payload: string }) => void) | undefined;
    mockListen.mockImplementation(
      (_event: string, handler: (event: { payload: string }) => void) => {
        tauriHandler = handler;
        return Promise.resolve(vi.fn());
      },
    );
    mockInvoke.mockResolvedValueOnce({
      general: { appearance: "dark" },
    });

    renderHook(() => useTheme());
    // Ensure theme was set to dark from prefs
    await vi.waitFor(() => {
      expect(useThemeStore.getState().mode).toBe("dark");
    });
    document.documentElement.classList.add("dark");

    act(() => {
      tauriHandler!({ payload: "light" });
    });

    // Should stay dark — mode is explicitly "dark"
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(useThemeStore.getState().resolved).toBe("dark");
  });
});
