import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useInfiniteQuery } from "@tanstack/react-query";
import { useSyncStore } from "@/store/syncStore";
import type { SyncItem, SyncActivityPage, SyncStateValue } from "@/types/sync";
import { STATUS_CONFIG, syncStateIcon } from "@/lib/statusColors";
import { formatSyncedTime } from "@/lib/formatTime";
import { formatSize } from "@/lib/formatSize";
import {
  Clock,
  MoreVertical,
  ExternalLink,
  Folder,
  FileText,
  Loader2,
} from "lucide-react";

// ─── Sync state i18n key mapping ─────────────────────────────────────────────

function syncStateKey(state: SyncStateValue): string {
  switch (state) {
    case "synced":
    case "cached":
    case "offline":
      return "sync_state.successfully_synced";
    case "uploading":
      return "sync_state.uploading";
    case "downloading":
      return "sync_state.downloading";
    case "modified":
      return "sync_state.modified";
    case "cloud_only":
      return "sync_state.cloud_only";
    case "error":
      return "sync_state.error";
    default:
      return "";
  }
}

// ─── Status header config ────────────────────────────────────────────────────

function FileIcon({ mimeType }: { mimeType: string }) {
  if (mimeType.startsWith("image/")) {
    return (
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" className="text-status-good" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
        <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
        <circle cx="8.5" cy="8.5" r="1.5" />
        <polyline points="21 15 16 10 5 21" />
      </svg>
    );
  }
  if (mimeType.includes("pdf")) {
    return (
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" className="text-status-danger" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
        <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" />
        <polyline points="14 2 14 8 20 8" />
      </svg>
    );
  }
  return <FileText className="h-5 w-5 text-app-text-secondary" />;
}

// (syncStateKey defined above)

function SyncStateIcon({ state }: { state: SyncStateValue }) {
  const result = syncStateIcon(state);
  if (!result) return null;
  const Icon = result.icon;
  return <Icon className={`h-4 w-4 shrink-0 ${result.className}`} />;
}

function isTransferring(state: SyncStateValue): boolean {
  return state === "uploading" || state === "downloading";
}

// ─── Sub-components ──────────────────────────────────────────────────────────

function StatusHeader() {
  const { t } = useTranslation();
  const status = useSyncStore((s) => s.status);
  const lastSyncedAt = useSyncStore((s) => s.lastSyncedAt);

  const config = STATUS_CONFIG[status] ?? STATUS_CONFIG["up-to-date"];
  const StatusIcon = config.icon;

  return (
    <div className="mb-6 flex items-center gap-3">
      <div className={`flex h-12 w-12 items-center justify-center rounded-full ${config.bgClass}`}>
        <StatusIcon className={`h-6 w-6 ${config.iconClass}`} />
      </div>
      <div>
        <h1 className="text-[20px] font-normal text-app-text-primary">
          {t(config.labelKey)}
        </h1>
        {lastSyncedAt && (
          <p className="flex items-center gap-1 text-[13px] text-app-text-secondary">
            <Clock className="h-3.5 w-3.5" />
            {formatSyncedTime(lastSyncedAt)}
          </p>
        )}
      </div>
    </div>
  );
}

function ProgressBar({ progress }: { progress: number }) {
  const pct = Math.round(progress * 100);
  return (
    <div className="flex items-center gap-2">
      <div className="h-1.5 w-16 overflow-hidden rounded-full bg-app-hover">
        <div
          className="h-full rounded-full bg-app-accent transition-all"
          style={{ width: `${pct}%` }}
        />
      </div>
      <span className="text-[11px] text-app-text-secondary">{pct}%</span>
    </div>
  );
}

// ─── Main page ───────────────────────────────────────────────────────────────

export default function SyncActivityPage() {
  const { t } = useTranslation();
  const [openMenuId, setOpenMenuId] = useState<string | null>(null);

  const {
    data,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
    isLoading,
  } = useInfiniteQuery({
    queryKey: ["sync-activity"],
    queryFn: ({ pageParam = 0 }) =>
      invoke<SyncActivityPage>("get_sync_activity", { page: pageParam }),
    getNextPageParam: (lastPage) => (lastPage.has_more ? lastPage.page + 1 : undefined),
    initialPageParam: 0,
    refetchInterval: 15_000,
  });

  const items: SyncItem[] = data?.pages.flatMap((p) => p.items) ?? [];

  const handleMenuAction = (item: SyncItem, action: string) => {
    setOpenMenuId(null);
    switch (action) {
      case "reveal":
        if (item.local_path) invoke("reveal_in_file_manager", { path: item.local_path });
        break;
      case "view-drive":
        if (item.drive_url) invoke("open_url", { url: item.drive_url });
        break;
      case "copy-link":
        if (item.drive_url) navigator.clipboard.writeText(item.drive_url);
        break;
    }
  };

  return (
    <div className="flex h-full flex-col">
      <StatusHeader />

      {/* Activity table */}
      <div className="flex-1 overflow-y-auto rounded-xl border border-app-border bg-app-surface">
        {/* Table header */}
        <div className="grid grid-cols-[1fr_120px_100px_40px] gap-4 border-b border-app-border px-5 py-3">
          <span className="text-[12px] font-medium uppercase tracking-wide text-app-text-secondary">
            {t("sync_activity.name")}
          </span>
          <span className="text-[12px] font-medium uppercase tracking-wide text-app-text-secondary">
            {t("sync_activity.file_size")}
          </span>
          <span className="text-[12px] font-medium uppercase tracking-wide text-app-text-secondary">
            {t("sync_activity.status")}
          </span>
          <span />
        </div>

        {/* Loading state */}
        {isLoading && (
          <div className="flex items-center justify-center py-12">
            <Loader2 className="h-5 w-5 animate-spin text-app-accent" />
          </div>
        )}

        {/* Empty state */}
        {!isLoading && items.length === 0 && (
          <div className="px-5 py-12 text-center text-[13px] text-app-text-secondary">
            {t("sync_activity.no_activity")}
          </div>
        )}

        {/* Table body */}
        {items.map((item, i) => {
          const fileId = item.file_id ?? `row-${i}`;
          const transferring = isTransferring(item.sync_state);

          return (
            <div
              key={fileId}
              className={`grid grid-cols-[1fr_120px_100px_40px] items-center gap-4 px-5 py-3 ${
                i < items.length - 1
                  ? "border-b border-app-subtle"
                  : ""
              }`}
            >
              {/* Name column */}
              <div className="flex items-center gap-3">
                <FileIcon mimeType={item.mime_type ?? ""} />
                <div className="min-w-0">
                  <p className="truncate text-[13px] font-medium text-app-text-primary">
                    {item.name}
                  </p>
                  <p className="text-[12px] text-app-text-secondary">
                    {t(syncStateKey(item.sync_state))}
                  </p>
                </div>
              </div>

              {/* File size column */}
              <span className="text-[13px] text-app-text-secondary">
                {formatSize(item.file_size) || "—"}
              </span>

              {/* Status column */}
              <div className="flex items-center justify-center">
                {transferring && item.progress != null ? (
                  <ProgressBar progress={item.progress} />
                ) : (
                  <SyncStateIcon state={item.sync_state} />
                )}
              </div>

              {/* Menu column */}
              <div className="relative flex justify-end">
                <button
                  className="flex h-7 w-7 items-center justify-center rounded-full text-app-text-secondary transition-colors hover:bg-app-hover"
                  onClick={() => setOpenMenuId(openMenuId === fileId ? null : fileId)}
                >
                  <MoreVertical className="h-4 w-4" />
                </button>
                {openMenuId === fileId && (
                  <>
                    <div className="fixed inset-0 z-10" onClick={() => setOpenMenuId(null)} />
                    <div className="absolute end-0 top-full z-20 w-48 rounded-lg border border-app-border bg-app-surface py-1 shadow-lg">
                      {item.local_path && (
                        <button
                          className="flex w-full items-center gap-2 px-4 py-2 text-[13px] text-app-text-primary transition-colors hover:bg-app-subtle"
                          onClick={() => handleMenuAction(item, "reveal")}
                        >
                          <Folder className="h-4 w-4 text-app-text-secondary" />
                          <span>{t("sync_activity.show_in_file_manager")}</span>
                        </button>
                      )}
                      {item.drive_url && (
                        <button
                          className="flex w-full items-center gap-2 px-4 py-2 text-[13px] text-app-text-primary transition-colors hover:bg-app-subtle"
                          onClick={() => handleMenuAction(item, "view-drive")}
                        >
                          <ExternalLink className="h-4 w-4 text-app-text-secondary" />
                          <span>{t("sync_activity.view_in_drive_web")}</span>
                        </button>
                      )}
                      {item.drive_url && (
                        <button
                          className="flex w-full items-center gap-2 px-4 py-2 text-[13px] text-app-text-primary transition-colors hover:bg-app-subtle"
                          onClick={() => handleMenuAction(item, "copy-link")}
                        >
                          <span className="w-4" />
                          <span>{t("sync_activity.copy_link")}</span>
                        </button>
                      )}
                    </div>
                  </>
                )}
              </div>
            </div>
          );
        })}

        {/* Load more */}
        {hasNextPage && (
          <div className="border-t border-app-subtle px-5 py-3 text-center">
            <button
              onClick={() => fetchNextPage()}
              disabled={isFetchingNextPage}
              className="text-[13px] font-medium text-app-accent hover:text-app-accent-hover disabled:opacity-50"
            >
              {isFetchingNextPage ? (
                <span className="flex items-center gap-1">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  {t("common.loading")}
                </span>
              ) : (
                t("sync_activity.load_more")
              )}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
