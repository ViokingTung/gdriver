import type { SyncStatusValue, SyncStateValue } from "@/types/sync";
import {
  Cloud,
  Loader2,
  Pause,
  AlertTriangle,
  WifiOff,
  Check,
  type LucideIcon,
} from "lucide-react";

// ─── Sync status card config ────────────────────────────────────────────────

export interface StatusConfig {
  icon: LucideIcon;
  iconClass: string;
  bgClass: string;
  labelKey: string;
}

export const STATUS_CONFIG: Record<SyncStatusValue, StatusConfig> = {
  "up-to-date": {
    icon: Cloud,
    iconClass: "text-status-good",
    bgClass: "bg-status-good-bg dark:bg-[var(--color-status-up-to-date-bg-dark)]",
    labelKey: "status.up_to_date",
  },
  syncing: {
    icon: Loader2,
    iconClass: "text-status-active animate-spin",
    bgClass: "bg-status-active-bg dark:bg-[var(--color-status-syncing-bg-dark)]",
    labelKey: "status.syncing",
  },
  paused: {
    icon: Pause,
    iconClass: "text-status-warn",
    bgClass: "bg-status-warn-bg dark:bg-[var(--color-status-paused-bg-dark)]",
    labelKey: "status.paused",
  },
  error: {
    icon: AlertTriangle,
    iconClass: "text-status-danger",
    bgClass: "bg-status-danger-bg dark:bg-[var(--color-status-error-bg-dark)]",
    labelKey: "status.error",
  },
  offline: {
    icon: WifiOff,
    iconClass: "text-status-neutral dark:text-[var(--color-status-offline-dark)]",
    bgClass: "bg-status-neutral-bg dark:bg-[var(--color-status-offline-bg-dark)]",
    labelKey: "status.offline",
  },
};

// ─── Per-file sync state icons ──────────────────────────────────────────────

export function syncStateIcon(state: SyncStateValue) {
  switch (state) {
    case "synced":
    case "cached":
    case "offline":
      return { icon: Check, className: "text-status-good" };
    case "uploading":
    case "downloading":
      return { icon: Loader2, className: "text-status-active animate-spin" };
    case "error":
      return { icon: AlertTriangle, className: "text-status-danger" };
    default:
      return null;
  }
}

// ─── File icon color classes ────────────────────────────────────────────────

export const FILE_ICON_COLORS = {
  image: "text-status-good",
  pdf: "text-status-danger",
  default: "text-app-text-secondary",
} as const;
