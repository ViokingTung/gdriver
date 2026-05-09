import { describe, it, expect, beforeEach } from "vitest";
import { useSyncStore } from "@/store/syncStore";

beforeEach(() => {
  useSyncStore.setState({
    status: "up-to-date",
    lastSyncedAt: null,
    currentSpeed: undefined,
    pendingCount: undefined,
  });
});

// ── Initial state ──────────────────────────────────────────────────────────

describe("initial state", () => {
  it("defaults to up-to-date", () => {
    expect(useSyncStore.getState().status).toBe("up-to-date");
  });

  it("has no last synced timestamp", () => {
    expect(useSyncStore.getState().lastSyncedAt).toBeNull();
  });

  it("has no current speed", () => {
    expect(useSyncStore.getState().currentSpeed).toBeUndefined();
  });

  it("has no pending count", () => {
    expect(useSyncStore.getState().pendingCount).toBeUndefined();
  });
});

// ── isPaused ───────────────────────────────────────────────────────────────

describe("isPaused", () => {
  it("returns false when up-to-date", () => {
    expect(useSyncStore.getState().isPaused()).toBe(false);
  });

  it("returns false when syncing", () => {
    useSyncStore.setState({ status: "syncing" });
    expect(useSyncStore.getState().isPaused()).toBe(false);
  });

  it("returns true when paused", () => {
    useSyncStore.setState({ status: "paused" });
    expect(useSyncStore.getState().isPaused()).toBe(true);
  });

  it("returns false when error", () => {
    useSyncStore.setState({ status: "error" });
    expect(useSyncStore.getState().isPaused()).toBe(false);
  });

  it("returns false when offline", () => {
    useSyncStore.setState({ status: "offline" });
    expect(useSyncStore.getState().isPaused()).toBe(false);
  });
});

// ── setStatus ──────────────────────────────────────────────────────────────

describe("setStatus", () => {
  it("updates status to syncing", () => {
    useSyncStore.getState().setStatus({ status: "syncing", ts: 1000 });
    expect(useSyncStore.getState().status).toBe("syncing");
  });

  it("updates status to paused", () => {
    useSyncStore.getState().setStatus({ status: "paused", ts: 2000 });
    expect(useSyncStore.getState().status).toBe("paused");
  });

  it("updates status to error", () => {
    useSyncStore.getState().setStatus({ status: "error", ts: 3000 });
    expect(useSyncStore.getState().status).toBe("error");
  });

  it("updates status to offline", () => {
    useSyncStore.getState().setStatus({ status: "offline", ts: 4000 });
    expect(useSyncStore.getState().status).toBe("offline");
  });

  it("updates speed and pending count", () => {
    useSyncStore
      .getState()
      .setStatus({ status: "syncing", ts: 1000, speed: 1024, pending: 5 });
    expect(useSyncStore.getState().currentSpeed).toBe(1024);
    expect(useSyncStore.getState().pendingCount).toBe(5);
  });

  it("records lastSyncedAt when transitioning to up-to-date", () => {
    useSyncStore
      .getState()
      .setStatus({ status: "up-to-date", ts: 1700000000000 });
    const last = useSyncStore.getState().lastSyncedAt;
    expect(last).toBeInstanceOf(Date);
    expect(last!.getTime()).toBe(1700000000000);
  });

  it("does NOT record lastSyncedAt for non-up-to-date statuses", () => {
    useSyncStore
      .getState()
      .setStatus({ status: "syncing", ts: 1700000000000 });
    expect(useSyncStore.getState().lastSyncedAt).toBeNull();
  });

  it("overwrites lastSyncedAt on subsequent up-to-date", () => {
    useSyncStore
      .getState()
      .setStatus({ status: "up-to-date", ts: 1000 });
    useSyncStore
      .getState()
      .setStatus({ status: "up-to-date", ts: 2000 });
    expect(useSyncStore.getState().lastSyncedAt!.getTime()).toBe(2000);
  });

  it("preserves lastSyncedAt when status changes to non-up-to-date", () => {
    useSyncStore
      .getState()
      .setStatus({ status: "up-to-date", ts: 1000 });
    useSyncStore
      .getState()
      .setStatus({ status: "syncing", ts: 2000 });
    expect(useSyncStore.getState().lastSyncedAt!.getTime()).toBe(1000);
  });

  it("handles undefined speed and pending", () => {
    useSyncStore.setState({ currentSpeed: 1024, pendingCount: 5 });
    useSyncStore
      .getState()
      .setStatus({ status: "syncing", ts: 1000 });
    expect(useSyncStore.getState().currentSpeed).toBeUndefined();
    expect(useSyncStore.getState().pendingCount).toBeUndefined();
  });

  it("handles all five status values", () => {
    const statuses = ["up-to-date", "syncing", "paused", "error", "offline"] as const;
    for (const st of statuses) {
      useSyncStore.getState().setStatus({ status: st, ts: 0 });
      expect(useSyncStore.getState().status).toBe(st);
    }
  });
});
