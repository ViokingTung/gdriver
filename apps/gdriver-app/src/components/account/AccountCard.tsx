import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useAccountStore } from "@/store/accountStore";
import { useAppStore } from "@/store/appStore";
import { DEFAULT_STORAGE_LIMIT_GB } from "@/lib/constants";
import { GOOGLE_DRIVE_HELP, GOOGLE_STORAGE } from "@/lib/urls";
import { X, Cloud, CircleHelp, Info, ChevronRight, Power, LogIn } from "lucide-react";

interface AccountCardProps {
  open: boolean;
  onClose: () => void;
}

function getInitial(name: string | null | undefined, email: string): string {
  if (name && name.length > 0) return name.charAt(0).toUpperCase();
  if (email && email.length > 0) return email.charAt(0).toUpperCase();
  return "?";
}

export default function AccountCard({ open, onClose }: AccountCardProps) {
  const { t } = useTranslation();
  const [imgError, setImgError] = useState(false);
  const [signingIn, setSigningIn] = useState(false);
  const account = useAccountStore((s) => s.activeAccount());
  const quota = useAccountStore((s) => s.activeQuota());
  const setOpenDialog = useAppStore((s) => s.setOpenDialog);

  if (!open) return null;

  // No account — show sign-in prompt
  if (!account) {
    const handleSignIn = async () => {
      setSigningIn(true);
      try {
        const authUrl = await invoke<string>("start_oauth_flow");
        await invoke("open_url", { url: authUrl });
      } catch {
        setSigningIn(false);
      }
    };

    return (
      <>
        <div className="fixed inset-0 z-10" onClick={onClose} />
        <div className="absolute end-0 top-full z-20 mt-1 w-80 overflow-hidden rounded-2xl border border-app-border bg-app-surface shadow-xl">
          <div className="relative px-6 py-8 text-center">
            <button
              className="absolute end-3 top-3 flex h-7 w-7 items-center justify-center rounded-full text-app-text-secondary transition-colors hover:bg-app-hover"
              onClick={onClose}
            >
              <X className="h-4 w-4" />
            </button>

            <div className="mx-auto mb-4 flex h-14 w-14 items-center justify-center rounded-full bg-app-hover">
              <LogIn className="h-6 w-6 text-app-text-secondary" />
            </div>

            <p className="mb-4 text-[14px] text-app-text-secondary">
              {t("account.sign_in_prompt")}
            </p>

            <button
              className="inline-flex items-center gap-2 rounded-full bg-app-accent px-6 py-2 text-[13px] font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-50"
              onClick={handleSignIn}
              disabled={signingIn}
            >
              {signingIn ? t("onboarding.signin.signing_in") : t("onboarding.signin.sign_in")}
            </button>
          </div>
        </div>
      </>
    );
  }

  const initial = getInitial(account.display_name, account.email);
  const greeting = account.display_name
    ? t("account.hi", { name: account.display_name })
    : t("account.hi_simple");

  // Storage calculations
  const usagePercent =
    quota?.limit && quota.limit > 0
      ? Math.round((quota.usage / quota.limit) * 100)
      : 0;
  const limitGb =
    quota?.limit && quota.limit > 0
      ? (quota.limit / (1024 * 1024 * 1024)).toFixed(2)
      : DEFAULT_STORAGE_LIMIT_GB;

  const handleQuit = () => {
    onClose();
    try {
      invoke("quit_app");
    } catch {
      window.close();
    }
  };

  const handleSyncOptions = () => {
    onClose();
    setOpenDialog("account-prefs");
  };

  const handleAbout = () => {
    onClose();
    setOpenDialog("about");
  };

  const handleHelp = () => {
    onClose();
    try {
      invoke("open_url", { url: GOOGLE_DRIVE_HELP });
    } catch {
      /* noop */
    }
  };

  const handleManageStorage = () => {
    try {
      invoke("open_url", { url: GOOGLE_STORAGE });
    } catch {
      /* noop */
    }
  };

  return (
    <>
      {/* Backdrop */}
      <div className="fixed inset-0 z-10" onClick={onClose} />

      {/* Card */}
      <div className="absolute end-0 top-full z-20 mt-1 w-80 overflow-hidden rounded-2xl border border-app-border bg-app-surface shadow-xl">
        {/* Top section — account info */}
        <div className="relative px-6 pb-5 pt-6 text-center">
          {/* Close button */}
          <button
            className="absolute end-3 top-3 flex h-7 w-7 items-center justify-center rounded-full text-app-text-secondary transition-colors hover:bg-app-hover"
            onClick={onClose}
          >
            <X className="h-4 w-4" />
          </button>

          {/* Avatar */}
          <div className="mx-auto mb-3 flex h-14 w-14 items-center justify-center overflow-hidden rounded-full bg-app-accent">
            {account.photo_url && !imgError ? (
              <img
                src={account.photo_url}
                alt={account.display_name ?? account.email}
                className="h-full w-full object-cover"
                onError={() => setImgError(true)}
              />
            ) : (
              <span className="text-xl font-medium text-white">{initial}</span>
            )}
          </div>

          {/* Greeting */}
          <p className="mb-0.5 text-[15px] font-medium text-app-text-primary">
            {greeting}
          </p>

          {/* Email */}
          <p className="mb-4 text-[12px] text-app-text-secondary">
            {account.email}
          </p>

          {/* Storage usage */}
          <div className="mb-1 flex items-center justify-between text-[12px] text-app-text-secondary">
            <span>
              {t("account.storage_usage", { percent: usagePercent, limit: limitGb })}
            </span>
          </div>
          <div className="mb-3 h-1 w-full overflow-hidden rounded-full bg-app-hover">
            <div
              className="h-full rounded-full bg-app-accent transition-all"
              style={{ width: `${Math.min(usagePercent, 100)}%` }}
            />
          </div>

          {/* Manage storage link */}
          <button
            className="text-[12px] font-medium text-app-accent hover:text-app-accent-hover"
            onClick={handleManageStorage}
          >
            {t("common.manage_storage")}
          </button>
        </div>

        {/* Bottom section — menu items */}
        <div className="bg-app-bg-secondary px-2 py-2">
          {/* Sync options */}
          <button
            className="flex w-full items-center gap-3 rounded-lg px-4 py-2.5 text-start transition-colors hover:bg-app-hover"
            onClick={handleSyncOptions}
          >
            <Cloud className="h-5 w-5 text-app-text-secondary" />
            <span className="flex-1 text-[13px] text-app-text-primary">
              {t("account.sync_options")}
            </span>
            <ChevronRight className="h-4 w-4 text-app-text-secondary" />
          </button>

          {/* Help */}
          <button
            className="flex w-full items-center gap-3 rounded-lg px-4 py-2.5 text-start transition-colors hover:bg-app-hover"
            onClick={handleHelp}
          >
            <CircleHelp className="h-5 w-5 text-app-text-secondary" />
            <span className="flex-1 text-[13px] text-app-text-primary">
              {t("account.help")}
            </span>
            <ChevronRight className="h-4 w-4 text-app-text-secondary" />
          </button>

          {/* About */}
          <button
            className="flex w-full items-center gap-3 rounded-lg px-4 py-2.5 text-start transition-colors hover:bg-app-hover"
            onClick={handleAbout}
          >
            <Info className="h-5 w-5 text-app-text-secondary" />
            <span className="flex-1 text-[13px] text-app-text-primary">
              {t("account.about")}
            </span>
            <ChevronRight className="h-4 w-4 text-app-text-secondary" />
          </button>

          {/* Separator */}
          <div className="mx-4 my-1 border-t border-app-border" />

          {/* Quit */}
          <button
            className="flex w-full items-center gap-3 rounded-lg px-4 py-2.5 text-start transition-colors hover:bg-status-danger-bg"
            onClick={handleQuit}
          >
            <Power className="h-5 w-5 text-status-danger dark:text-[var(--color-status-error-dark)]" />
            <span className="text-[13px] text-status-danger dark:text-[var(--color-status-error-dark)]">
              {t("account.quit_gdrive")}
            </span>
          </button>
        </div>
      </div>
    </>
  );
}
