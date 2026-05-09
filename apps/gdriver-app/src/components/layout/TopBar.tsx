import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useSyncStore } from "@/store/syncStore";
import { useAppStore } from "@/store/appStore";
import { useAccountStore } from "@/store/accountStore";
import { GOOGLE_DRIVE_HELP } from "@/lib/urls";
import DriveLogo from "@/components/DriveLogo";
import AccountCard from "@/components/account/AccountCard";
import {
  Search,
  Pause,
  Play,
  Settings,
} from "lucide-react";

export default function TopBar() {
  const { t } = useTranslation();
  const [searchFocused, setSearchFocused] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [accountOpen, setAccountOpen] = useState(false);
  const [imgError, setImgError] = useState(false);
  const isPaused = useSyncStore((s) => s.status === "paused");
  const setOpenDialog = useAppStore((s) => s.setOpenDialog);
  const account = useAccountStore((s) => s.activeAccount());

  const handleToggleSync = () => {
    const cmd = isPaused ? "resume_sync" : "pause_sync";
    invoke(cmd).catch(() => {});
  };

  const handleQuit = () => {
    try { invoke("quit_app"); } catch { window.close(); }
  };

  return (
    <header className="flex h-14 shrink-0 items-center gap-2 border-b border-app-border bg-app-bg-primary px-4">
      {/* Logo + Drive text */}
      <div className="flex items-center gap-1.5 pe-4">
        <DriveLogo size={32} />
        <span className="text-lg font-normal text-app-text-primary select-none">
          Drive
        </span>
      </div>

      {/* Search box */}
      <div
        className={`flex flex-1 items-center gap-2 rounded-full border px-4 py-1.5 transition-colors ${
          searchFocused
            ? "border-app-accent bg-app-surface shadow-[0_1px_6px_rgba(32,33,36,0.18)]"
            : "border-app-border bg-app-subtle"
        }`}
      >
        <Search className="h-4 w-4 shrink-0 text-app-text-secondary" />
        <input
          type="text"
          placeholder={t("topbar.search_placeholder")}
          className="min-w-0 flex-1 bg-transparent text-[13px] text-app-text-primary placeholder-app-text-secondary outline-none"
          onFocus={() => setSearchFocused(true)}
          onBlur={() => setSearchFocused(false)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              const q = encodeURIComponent((e.target as HTMLInputElement).value);
              try {
                invoke("open_url", { url: `https://drive.google.com/search?q=${q}` });
              } catch { /* M8 */ }
            }
          }}
        />
      </div>

      {/* Pause / Resume sync */}
      <button
        className="flex h-9 w-9 items-center justify-center rounded-full text-app-text-secondary transition-colors hover:bg-app-hover"
        title={isPaused ? t("topbar.resume_sync") : t("topbar.pause_sync")}
        onClick={handleToggleSync}
      >
        {isPaused ? <Play className="h-5 w-5" /> : <Pause className="h-5 w-5" />}
      </button>

      {/* Settings */}
      <div className="relative">
        <button
          className="flex h-9 w-9 items-center justify-center rounded-full text-app-text-secondary transition-colors hover:bg-app-hover"
          title={t("topbar.settings")}
          onClick={() => setSettingsOpen(!settingsOpen)}
        >
          <Settings className="h-5 w-5" />
        </button>

        {settingsOpen && (
          <>
            <div className="fixed inset-0 z-10" onClick={() => setSettingsOpen(false)} />
            <div className="absolute end-0 top-full z-20 mt-1 w-56 rounded-lg border border-app-border bg-app-surface py-1 shadow-lg">
              {[
                { label: t("topbar.menu_preferences"), action: () => setOpenDialog("preferences") },
                { label: t("topbar.menu_offline_files"), action: () => setOpenDialog("offline-files") },
                { label: t("topbar.menu_error_list"), action: () => setOpenDialog("error-list") },
                { label: t("topbar.menu_about"), action: () => setOpenDialog("about") },
                { label: t("topbar.menu_help"), action: () => { try { invoke("open_url", { url: GOOGLE_DRIVE_HELP }); } catch {} } },
                { label: t("topbar.menu_send_feedback"), action: () => setOpenDialog("feedback") },
                { separator: true },
                { label: t("topbar.menu_quit"), action: handleQuit, danger: true },
              ].map((item, i) =>
                "separator" in item && item.separator ? (
                  <div key={i} className="my-1 border-t border-app-border" />
                ) : (
                  "separator" in item ? null : (
                    <button
                      key={item.label}
                      className={`w-full px-4 py-2 text-start text-[13px] transition-colors hover:bg-app-subtle ${
                        "danger" in item && item.danger
                          ? "text-status-danger dark:text-[var(--color-status-error-dark)]"
                          : "text-app-text-primary"
                      }`}
                      onClick={() => { item.action(); setSettingsOpen(false); }}
                    >
                      {item.label}
                    </button>
                  )
                )
              )}
            </div>
          </>
        )}
      </div>

      {/* Account avatar */}
      <div className="relative">
        <button
          className="flex h-8 w-8 items-center justify-center overflow-hidden rounded-full bg-app-accent text-sm font-medium text-white transition-opacity hover:opacity-90"
          title={t("topbar.account")}
          onClick={() => setAccountOpen(!accountOpen)}
        >
          {account?.photo_url && !imgError ? (
            <img
              src={account.photo_url}
              alt=""
              className="h-full w-full object-cover"
              onError={() => setImgError(true)}
            />
          ) : (
            <span>
              {account?.display_name?.charAt(0)?.toUpperCase() ??
                account?.email?.charAt(0)?.toUpperCase() ??
                "?"}
            </span>
          )}
        </button>

        <AccountCard
          open={accountOpen}
          onClose={() => setAccountOpen(false)}
        />
      </div>
    </header>
  );
}
