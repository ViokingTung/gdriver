import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { useAppStore } from "@/store/appStore";
import { GOOGLE_DRIVE_OFFLINE_HELP, GOOGLE_DRIVE_WEB, GOOGLE_DRIVE_HELP, GOOGLE_DRIVE_SHARED_WITH_ME } from "@/lib/urls";
import { useSyncStore } from "@/store/syncStore";
import type { SyncItem, SyncStateValue, Notification } from "@/types/sync";
import { STATUS_CONFIG, syncStateIcon } from "@/lib/statusColors";
import { formatSyncedTime } from "@/lib/formatTime";
import {
  Clock,
  MoreVertical,
  ExternalLink,
  Folder,
  Globe,
  BookOpen,
  HelpCircle,
  FileText,
  Plus,
  AlertTriangle,
} from "lucide-react";

function formatSpeed(bytesPerSec: number): string {
  if (bytesPerSec < 1024) return `${bytesPerSec} B/s`;
  if (bytesPerSec < 1024 * 1024) return `${(bytesPerSec / 1024).toFixed(1)} KB/s`;
  return `${(bytesPerSec / (1024 * 1024)).toFixed(1)} MB/s`;
}

// ─── File icon helper ──────────────────────────────────────────────────────────

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

// ─── Sub-components ────────────────────────────────────────────────────────────

function SyncStatusCard() {
  const { t } = useTranslation();
  const status = useSyncStore((s) => s.status);
  const lastSyncedAt = useSyncStore((s) => s.lastSyncedAt);
  const currentSpeed = useSyncStore((s) => s.currentSpeed);
  const pendingCount = useSyncStore((s) => s.pendingCount);

  const config = STATUS_CONFIG[status];
  const StatusIcon = config.icon;

  let detail: string;
  if (status === "up-to-date" && lastSyncedAt) {
    detail = formatSyncedTime(lastSyncedAt);
  } else if (status === "syncing") {
    const parts: string[] = [];
    if (currentSpeed && currentSpeed > 0) parts.push(formatSpeed(currentSpeed));
    if (pendingCount && pendingCount > 0) parts.push(t("home.items_pending", { count: pendingCount }));
    detail = parts.length > 0 ? parts.join(" · ") : t("home.syncing_your_files");
  } else if (status === "error") {
    detail = t("home.some_items_failed");
  } else {
    detail = "";
  }

  return (
    <div className="rounded-xl border border-app-border bg-app-surface p-5">
      <div className="mb-3 flex items-center gap-3">
        <div className={`flex h-10 w-10 items-center justify-center rounded-full ${config.bgClass}`}>
          <StatusIcon className={`h-5 w-5 ${config.iconClass}`} />
        </div>
        <div>
          <p className="text-[14px] font-medium text-app-text-primary">
            {t(config.labelKey)}
          </p>
          {detail && (
            <p className="flex items-center gap-1 text-[12px] text-app-text-secondary">
              <Clock className="h-3 w-3" />
              {detail}
            </p>
          )}
        </div>
      </div>
    </div>
  );
}

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

function SyncStateIcon({ state }: { state: SyncStateValue }) {
  const result = syncStateIcon(state);
  if (!result) return null;
  const Icon = result.icon;
  return <Icon className={`h-4 w-4 shrink-0 ${result.className}`} />;
}

function RecentFilesList() {
  const { t } = useTranslation();
  const [openMenuId, setOpenMenuId] = useState<string | null>(null);

  const { data: items = [] } = useQuery<SyncItem[]>({
    queryKey: ["recent-sync-items"],
    queryFn: () => invoke<SyncItem[]>("get_recent_sync_items", { limit: 10 }),
    refetchInterval: 30_000,
  });

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
    <div className="mt-4 rounded-xl border border-app-border bg-app-surface">
      <div className="border-b border-app-border px-5 py-3">
        <h2 className="text-[12px] font-medium uppercase tracking-wide text-app-text-secondary">
          {t("home.recent_files")}
        </h2>
      </div>
      {items.length === 0 ? (
        <div className="px-5 py-8 text-center text-[13px] text-app-text-secondary">
          {t("home.no_recent_files")}
        </div>
      ) : (
        items.map((item, i) => (
          <div
            key={item.file_id ?? i}
            className={`flex items-center gap-3 px-5 py-3 ${
              i < items.length - 1
                ? "border-b border-app-subtle"
                : ""
            }`}
          >
            <FileIcon mimeType={item.mime_type ?? ""} />
            <div className="min-w-0 flex-1">
              <p className="truncate text-[13px] font-medium text-app-text-primary">
                {item.name}
              </p>
              <p className="text-[12px] text-app-text-secondary">
                {t(syncStateKey(item.sync_state))}
              </p>
            </div>
            <SyncStateIcon state={item.sync_state} />
            <div className="relative">
              <button
                className="flex h-7 w-7 items-center justify-center rounded-full text-app-text-secondary transition-colors hover:bg-app-hover"
                onClick={() => setOpenMenuId(openMenuId === item.file_id ? null : (item.file_id ?? null))}
              >
                <MoreVertical className="h-4 w-4" />
              </button>
              {openMenuId === item.file_id && (
                <>
                  <div className="fixed inset-0 z-10" onClick={() => setOpenMenuId(null)} />
                  <div className="absolute end-0 top-full z-20 w-48 rounded-lg border border-app-border bg-app-surface py-1 shadow-lg">
                    {item.local_path && (
                      <button
                        className="flex w-full items-center gap-2 px-4 py-2 text-[13px] text-app-text-primary transition-colors hover:bg-app-subtle"
                        onClick={() => handleMenuAction(item, "reveal")}
                      >
                        <Folder className="h-4 w-4 text-app-text-secondary" />
                        <span>{t("home.show_in_file_manager")}</span>
                      </button>
                    )}
                    {item.drive_url && (
                      <button
                        className="flex w-full items-center gap-2 px-4 py-2 text-[13px] text-app-text-primary transition-colors hover:bg-app-subtle"
                        onClick={() => handleMenuAction(item, "view-drive")}
                      >
                        <ExternalLink className="h-4 w-4 text-app-text-secondary" />
                        <span>{t("home.view_in_drive_web")}</span>
                      </button>
                    )}
                    {item.drive_url && (
                      <button
                        className="flex w-full items-center gap-2 px-4 py-2 text-[13px] text-app-text-primary transition-colors hover:bg-app-subtle"
                        onClick={() => handleMenuAction(item, "copy-link")}
                      >
                        <span className="w-4" />
                        <span>{t("home.copy_link")}</span>
                      </button>
                    )}
                  </div>
                </>
              )}
            </div>
          </div>
        ))
      )}
      <div className="border-t border-app-subtle px-5 py-2">
        <button
          onClick={() => useAppStore.getState().setCurrentPage("sync")}
          className="text-[13px] font-medium text-app-accent hover:text-app-accent-hover"
        >
          {t("common.view_all")}
        </button>
      </div>
    </div>
  );
}

function SharedFilesCard() {
  const { t } = useTranslation();
  return (
    <div className="mt-4 rounded-xl border border-app-border bg-app-surface p-5">
      <h2 className="mb-1 text-[14px] font-medium text-app-text-primary">
        {t("home.shared_files_title")}
      </h2>
      <p className="mb-3 text-[12px] text-app-text-secondary">
        {t("home.shared_files_desc")}
      </p>
      <button
        className="text-[13px] font-medium text-app-accent hover:text-app-accent-hover"
        onClick={() => {
          try { invoke("open_url", { url: GOOGLE_DRIVE_SHARED_WITH_ME }); } catch {}
        }}
      >
        {t("common.view_all")}
      </button>
    </div>
  );
}

function NotificationsSummary() {
  const { t } = useTranslation();
  const { data: notifications = [] } = useQuery<Notification[]>({
    queryKey: ["notifications-summary"],
    queryFn: () => invoke<Notification[]>("get_notifications", { unreadOnly: true, limit: 3 }),
    refetchInterval: 30_000,
  });

  if (notifications.length === 0) {
    return (
      <div className="rounded-xl border border-app-border bg-app-bg-secondary p-6 text-center">
        <div className="mx-auto mb-4 flex h-20 w-20 items-center justify-center">
          <svg width="72" height="72" viewBox="0 0 72 72" fill="none" xmlns="http://www.w3.org/2000/svg">
            <circle cx="36" cy="36" r="28" className="fill-illust-bg" />
            <path d="M28 42h16M28 42l2-12h12l2 12M28 42c0 0 1 4 8 4s8-4 8-4" className="stroke-illust-stroke" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
            <path d="M32 18l-4 4-4-4" className="stroke-illust-accent" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </div>
        <h3 className="mb-1 text-[14px] font-medium text-app-text-primary">
          {t("home.caught_up")}
        </h3>
        <p className="text-[12px] text-app-text-secondary">
          {t("home.caught_up_desc")}
        </p>
      </div>
    );
  }

  // Transform to a flat display shape so TypeScript narrowing works in JSX.
  const rows = notifications.map((n) => {
    if (n.type === "conflict") {
      return { id: n.id, isWarning: false, title: n.file_name, subtitle: t("home.conflict_created") };
    }
    if (n.type === "storage_warning") {
      return { id: n.id, isWarning: true, title: t("home.storage_warning", { percent: Math.round(n.usage_percent) }), subtitle: t("home.storage_running_low") };
    }
    return { id: n.id, isWarning: true, title: n.file_name ?? t("home.sync_error"), subtitle: n.error_msg };
  });

  return (
    <div className="rounded-xl border border-app-border bg-app-surface">
      <div className="border-b border-app-border px-5 py-3">
        <h2 className="text-[12px] font-medium uppercase tracking-wide text-app-text-secondary">
          {t("home.notifications")}
          <span className="ms-2 inline-flex h-4 min-w-[16px] items-center justify-center rounded-full bg-status-danger px-1 text-[10px] font-medium text-white">
            {rows.length}
          </span>
        </h2>
      </div>
      {rows.map((r) => (
        <div
          key={r.id}
          className="flex items-center gap-3 px-5 py-3"
        >
          <div className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-full ${
            r.isWarning
              ? "bg-status-danger-bg dark:bg-[var(--color-status-error-bg-dark)]"
              : "bg-status-warn-bg dark:bg-[var(--color-status-paused-bg-dark)]"
          }`}>
            {r.isWarning ? (
              <AlertTriangle className="h-3.5 w-3.5 text-status-danger" />
            ) : (
              <AlertTriangle className="h-3.5 w-3.5 text-status-warn" />
            )}
          </div>
          <div className="min-w-0 flex-1">
            <p className="truncate text-[13px] font-medium text-app-text-primary">
              {r.title}
            </p>
            <p className="truncate text-[12px] text-app-text-secondary">
              {r.subtitle}
            </p>
          </div>
        </div>
      ))}
      <div className="border-t border-app-subtle px-5 py-2">
        <button
          onClick={() => useAppStore.getState().setCurrentPage("notifications")}
          className="text-[13px] font-medium text-app-accent hover:text-app-accent-hover"
        >
          {t("common.view_all")}
        </button>
      </div>
    </div>
  );
}

function QuickLinks() {
  const { t } = useTranslation();
  const setOpenDialog = useAppStore((s) => s.setOpenDialog);

  const links = [
    {
      label: t("home.add_more_folders"),
      icon: Plus,
      action: () => setOpenDialog("preferences"),
    },
    {
      label: t("home.open_drive_web"),
      icon: Globe,
      action: () => invoke("open_url", { url: GOOGLE_DRIVE_WEB }).catch(() => {}),
    },
    {
      label: t("home.learn_offline"),
      icon: BookOpen,
      action: () => invoke("open_url", { url: GOOGLE_DRIVE_OFFLINE_HELP }).catch(() => {}),
    },
    {
      label: t("home.faq"),
      icon: HelpCircle,
      action: () => invoke("open_url", { url: GOOGLE_DRIVE_HELP }).catch(() => {}),
    },
  ];

  return (
    <div className="mt-4 rounded-xl border border-app-border bg-app-surface">
      <div className="border-b border-app-border px-5 py-3">
        <h2 className="text-[12px] font-medium uppercase tracking-wide text-app-text-secondary">
          {t("home.quick_links")}
        </h2>
      </div>
      {links.map((link, i) => (
        <button
          key={link.label}
          onClick={link.action}
          className={`flex w-full items-center gap-3 px-5 py-3 text-start text-[13px] font-medium text-app-accent transition-colors hover:bg-app-bg-secondary ${
            i < links.length - 1 ? "border-b border-app-subtle" : ""
          }`}
        >
          <link.icon className="h-4 w-4 shrink-0 text-app-text-secondary" />
          {link.label}
        </button>
      ))}
    </div>
  );
}

// ─── HomePage ──────────────────────────────────────────────────────────────────

export default function HomePage() {
  return (
    <div className="flex h-full gap-6">
      {/* Left column */}
      <div className="flex-1 overflow-y-auto pb-8">
        <SyncStatusCard />
        <RecentFilesList />
        <SharedFilesCard />
      </div>

      {/* Right column */}
      <div className="w-[296px] shrink-0 overflow-y-auto pb-8">
        <NotificationsSummary />
        <QuickLinks />
      </div>
    </div>
  );
}
