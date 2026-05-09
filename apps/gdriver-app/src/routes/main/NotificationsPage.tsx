import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { Notification } from "@/types/sync";
import { formatRelativeTime } from "@/lib/formatTime";
import { GOOGLE_STORAGE } from "@/lib/urls";
import {
  AlertTriangle,
  HardDrive,
  RefreshCw,
  X,
  Folder,
  Check,
  Loader2,
} from "lucide-react";

// ─── Notification cards ──────────────────────────────────────────────────────

function ConflictCard({
  n,
  onDismiss,
}: {
  n: Extract<Notification, { type: "conflict" }>;
  onDismiss: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex items-start gap-3 rounded-lg border border-app-border bg-app-surface p-4">
      <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-status-warn-bg dark:bg-[var(--color-status-paused-bg-dark)]">
        <AlertTriangle className="h-4.5 w-4.5 text-status-warn" />
      </div>
      <div className="min-w-0 flex-1">
        <p className="text-[13px] font-medium text-app-text-primary">
          {n.file_name}
        </p>
        <p className="text-[12px] text-app-text-secondary">
          {t("notifications_page.conflict_created", { name: n.conflict_copy_name })}
        </p>
        <p className="mt-1 text-[11px] text-app-text-muted">
          {formatRelativeTime(n.created_at)}
        </p>
        <button
          className="mt-2 flex items-center gap-1 text-[12px] font-medium text-app-accent hover:text-app-accent-hover"
          onClick={() => {
            invoke("open_url", {
              url: `https://drive.google.com/file/d/${n.file_id ?? ""}/view`,
            }).catch(() => {});
          }}
        >
          <Folder className="h-3.5 w-3.5" />
          {t("notifications_page.view_in_drive")}
        </button>
      </div>
      <button
        onClick={onDismiss}
        className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-app-text-secondary transition-colors hover:bg-app-hover"
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}

function StorageWarningCard({
  n,
  onDismiss,
}: {
  n: Extract<Notification, { type: "storage_warning" }>;
  onDismiss: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex items-start gap-3 rounded-lg border border-app-border bg-app-surface p-4">
      <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-status-danger-bg dark:bg-[var(--color-status-error-bg-dark)]">
        <HardDrive className="h-4.5 w-4.5 text-status-danger" />
      </div>
      <div className="min-w-0 flex-1">
        <p className="text-[13px] font-medium text-app-text-primary">
          {t("notifications_page.storage_almost_full")}
        </p>
        <p className="text-[12px] text-app-text-secondary">
          {t("notifications_page.storage_usage", { percent: Math.round(n.usage_percent) })}
        </p>
        <p className="mt-1 text-[11px] text-app-text-muted">
          {formatRelativeTime(n.created_at)}
        </p>
        <button
          className="mt-2 flex items-center gap-1 text-[12px] font-medium text-app-accent hover:text-app-accent-hover"
          onClick={() => {
            invoke("open_url", { url: GOOGLE_STORAGE }).catch(
              () => {}
            );
          }}
        >
          {t("notifications_page.manage_storage")}
        </button>
      </div>
      <button
        onClick={onDismiss}
        className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-app-text-secondary transition-colors hover:bg-app-hover"
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}

function SyncErrorCard({
  n,
  onDismiss,
  onRetry,
  retrying,
}: {
  n: Extract<Notification, { type: "sync_error" }>;
  onDismiss: () => void;
  onRetry: () => void;
  retrying: boolean;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex items-start gap-3 rounded-lg border border-app-border bg-app-surface p-4">
      <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-status-danger-bg dark:bg-[var(--color-status-error-bg-dark)]">
        <AlertTriangle className="h-4.5 w-4.5 text-status-danger" />
      </div>
      <div className="min-w-0 flex-1">
        <p className="text-[13px] font-medium text-app-text-primary">
          {n.file_name ?? t("notifications_page.sync_error")}
        </p>
        <p className="text-[12px] text-app-text-secondary">
          {n.error_msg}
        </p>
        <p className="mt-1 text-[11px] text-app-text-muted">
          {formatRelativeTime(n.created_at)}
        </p>
        <div className="mt-2 flex gap-2">
          <button
            disabled={retrying}
            onClick={onRetry}
            className="flex items-center gap-1 rounded-md bg-app-accent px-3 py-1.5 text-[12px] font-medium text-white transition-colors hover:bg-app-accent-hover disabled:opacity-50"
          >
            {retrying ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" />
            )}
            {t("common.retry")}
          </button>
          <button
            onClick={onDismiss}
            className="flex items-center gap-1 rounded-md border border-app-border px-3 py-1.5 text-[12px] font-medium text-app-text-secondary transition-colors hover:bg-app-subtle"
          >
            <Check className="h-3.5 w-3.5" />
            {t("common.dismiss")}
          </button>
        </div>
      </div>
      <button
        onClick={onDismiss}
        className="flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-app-text-secondary transition-colors hover:bg-app-hover"
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}

// ─── Main page ───────────────────────────────────────────────────────────────

export default function NotificationsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [retryingId, setRetryingId] = useState<number | null>(null);

  const { data: notifications = [], isLoading } = useQuery<Notification[]>({
    queryKey: ["notifications"],
    queryFn: () => invoke<Notification[]>("get_notifications", { limit: 100 }),
    refetchInterval: 30_000,
  });

  const handleDismiss = async (id: number) => {
    await invoke("dismiss_notification", { id });
    queryClient.invalidateQueries({ queryKey: ["notifications"] });
    queryClient.invalidateQueries({ queryKey: ["notifications-summary"] });
  };

  const handleMarkAllRead = async () => {
    await invoke("mark_all_notifications_read");
    queryClient.invalidateQueries({ queryKey: ["notifications"] });
    queryClient.invalidateQueries({ queryKey: ["notifications-summary"] });
  };

  const handleRetry = async (errorId: number, notificationId: number) => {
    setRetryingId(notificationId);
    try {
      await invoke("retry_sync_error", { errorId });
      await handleDismiss(notificationId);
    } catch {
      // Error already surfaced in the UI
    } finally {
      setRetryingId(null);
    }
  };

  if (isLoading) {
    return (
      <div className="flex h-full items-center justify-center">
        <Loader2 className="h-5 w-5 animate-spin text-app-accent" />
      </div>
    );
  }

  if (notifications.length === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center">
        {/* Empty state illustration */}
        <div className="mb-6">
          <svg width="120" height="120" viewBox="0 0 120 120" fill="none" xmlns="http://www.w3.org/2000/svg">
            <circle cx="60" cy="60" r="48" className="fill-illust-bg" />
            <circle cx="60" cy="52" r="12" className="fill-app-text-secondary" />
            <path d="M48 82c0 0 4-12 12-12s12 12 12 12" className="stroke-app-text-secondary" strokeWidth="2" strokeLinecap="round" />
            <path d="M72 56c4 4 8 8 12 6" className="stroke-app-text-secondary" strokeWidth="2" strokeLinecap="round" />
            <circle cx="86" cy="60" r="5" className="fill-status-warn" />
            <circle cx="82" cy="56" r="4" className="fill-status-warn" />
            <circle cx="90" cy="56" r="4" className="fill-status-warn" />
            <circle cx="84" cy="52" r="4" className="fill-status-warn" />
            <circle cx="88" cy="52" r="4" className="fill-status-warn" />
            <circle cx="86" cy="58" r="2" className="fill-status-danger" />
            <path d="M86 63c0 0 1 3-2 5" className="stroke-illust-accent" strokeWidth="1.5" strokeLinecap="round" />
            <path d="M42 94c-2-1-3 0-2 2" className="stroke-illust-accent" strokeWidth="1.5" strokeLinecap="round" />
            <path d="M78 95c2-1 3 0 2 2" className="stroke-illust-accent" strokeWidth="1.5" strokeLinecap="round" />
            <path d="M36 98h48" className="stroke-app-border" strokeWidth="2" strokeLinecap="round" />
          </svg>
        </div>
        <h2 className="mb-2 text-[20px] font-normal text-app-text-primary">
          {t("notifications_page.caught_up")}
        </h2>
        <p className="max-w-xs text-center text-[13px] leading-relaxed text-app-text-secondary">
          {t("notifications_page.caught_up_desc")}
        </p>
      </div>
    );
  }

  const unreadCount = notifications.filter((n) => !n.is_read).length;

  return (
    <div className="flex h-full flex-col">
      {/* Header */}
      <div className="mb-4 flex items-center justify-between">
        <h1 className="text-[20px] font-normal text-app-text-primary">
          {t("notifications_page.title")}
          {unreadCount > 0 && (
            <span className="ms-2 inline-flex h-5 min-w-[20px] items-center justify-center rounded-full bg-status-danger px-1.5 text-[11px] font-medium text-white">
              {unreadCount}
            </span>
          )}
        </h1>
        {unreadCount > 0 && (
          <button
            onClick={handleMarkAllRead}
            className="text-[13px] font-medium text-app-accent hover:text-app-accent-hover"
          >
            {t("notifications_page.mark_all_read")}
          </button>
        )}
      </div>

      {/* Notification list */}
      <div className="flex-1 space-y-3 overflow-y-auto pb-4">
        {notifications.map((n) => {
          switch (n.type) {
            case "conflict":
              return (
                <ConflictCard
                  key={n.id}
                  n={n}
                  onDismiss={() => handleDismiss(n.id)}
                />
              );
            case "storage_warning":
              return (
                <StorageWarningCard
                  key={n.id}
                  n={n}
                  onDismiss={() => handleDismiss(n.id)}
                />
              );
            case "sync_error":
              return (
                <SyncErrorCard
                  key={n.id}
                  n={n}
                  onDismiss={() => handleDismiss(n.id)}
                  onRetry={() => handleRetry(n.error_id, n.id)}
                  retrying={retryingId === n.id}
                />
              );
            default:
              return null;
          }
        })}
      </div>
    </div>
  );
}
