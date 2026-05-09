import { create } from "zustand";
import type { SyncStatusValue, SyncStatusPayload } from "@/types/sync";

interface SyncState {
  status: SyncStatusValue;
  lastSyncedAt: Date | null;
  currentSpeed?: number;
  pendingCount?: number;

  /** Apply a status push from the daemon. */
  setStatus: (payload: SyncStatusPayload) => void;

  /** Whether the engine is currently paused. */
  isPaused: () => boolean;
}

export const useSyncStore = create<SyncState>((set, get) => ({
  status: "up-to-date",
  lastSyncedAt: null,
  currentSpeed: undefined,
  pendingCount: undefined,

  setStatus: (payload) => {
    const update: Partial<SyncState> = {
      status: payload.status,
      currentSpeed: payload.speed,
      pendingCount: payload.pending,
    };
    // Record the last-synced timestamp when transitioning to up-to-date.
    if (payload.status === "up-to-date") {
      update.lastSyncedAt = new Date(payload.ts);
    }
    set(update);
  },

  isPaused: () => get().status === "paused",
}));
