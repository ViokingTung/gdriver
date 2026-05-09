import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import i18next from "i18next";
import { useAccountStore } from "@/store/accountStore";
import { useAppStore } from "@/store/appStore";
import { applyThemeFromMode } from "@/store/themeStore";
import { formatSize } from "@/lib/formatSize";
import { DEFAULT_STORAGE_LIMIT_GB, DEFAULT_RATE_LIMIT_KBPS, DEFAULT_SEARCH_HOTKEY } from "@/lib/constants";
import { GOOGLE_STORAGE } from "@/lib/urls";
import {
  Dialog,
  DialogHeader,
  DialogContent,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import {
  Monitor,
  Cloud,
  HardDrive,
  Image,
  Trash2,
  ChevronDown,
  FolderOpen,
} from "lucide-react";

// ─── Types mirroring the Rust Preferences struct ─────────────────────────────

interface GeneralPrefs {
  launch_on_login: boolean;
  appearance: "light" | "dark" | "follow_system";
  language: string;
  prompt_backup_devices: boolean;
}

interface NetworkPrefs {
  proxy: string;
  download_rate_limit: number;
  upload_rate_limit: number;
}

interface HotkeyPrefs {
  search_enabled: boolean;
  search_key: string;
}

interface TelemetryPrefs {
  auto_send_diagnostics: boolean;
}

interface VfsPrefs {
  mount_point: string;
  sync_mode: "stream" | "mirror";
}

interface Preferences {
  general: GeneralPrefs;
  network: NetworkPrefs;
  hotkeys: HotkeyPrefs;
  telemetry: TelemetryPrefs;
  vfs: VfsPrefs;
}

interface SyncFolder {
  id: string;
  name: string;
  path: string;
  type: "drive" | "photos";
  size?: number;
}

const DEFAULT_PREFS: Preferences = {
  general: {
    launch_on_login: true,
    appearance: "follow_system",
    language: "follow_account",
    prompt_backup_devices: true,
  },
  network: { proxy: "auto", download_rate_limit: 0, upload_rate_limit: 0 },
  hotkeys: { search_enabled: true, search_key: DEFAULT_SEARCH_HOTKEY },
  telemetry: { auto_send_diagnostics: true },
  vfs: { mount_point: "~/GoogleDrive", sync_mode: "stream" },
};

const LANGUAGES = [
  { value: "follow_account", labelKey: "common.follow_google_account" },
  { value: "en", label: "English" },
  { value: "zh-CN", label: "简体中文" },
  { value: "zh-TW", label: "繁體中文" },
  { value: "ja", label: "日本語" },
  { value: "ko", label: "한국어" },
  { value: "de", label: "Deutsch" },
  { value: "fr", label: "Français" },
  { value: "es", label: "Español" },
  { value: "pt-BR", label: "Português (Brasil)" },
  { value: "ru", label: "Русский" },
  { value: "it", label: "Italiano" },
  { value: "ar", label: "العربية" },
];

// ─── Section header ──────────────────────────────────────────────────────────

function SectionHeading({
  icon,
  title,
  subtitle,
}: {
  icon: React.ReactNode;
  title: string;
  subtitle?: string;
}) {
  return (
    <div className="mb-4 flex items-start gap-3">
      <div className="mt-0.5 text-app-text-secondary">{icon}</div>
      <div>
        <h3 className="text-[15px] font-medium text-app-text-primary">
          {title}
        </h3>
        {subtitle && (
          <p className="mt-0.5 text-[12px] text-app-text-secondary">
            {subtitle}
          </p>
        )}
      </div>
    </div>
  );
}

// ─── Checkbox row ────────────────────────────────────────────────────────────

function CheckRow({
  label,
  description,
  checked,
  onChange,
}: {
  label: string;
  description?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="flex cursor-pointer items-start gap-3 py-2">
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="mt-0.5 h-4 w-4 shrink-0 accent-app-accent"
      />
      <div>
        <p className="text-[13px] text-app-text-primary">
          {label}
        </p>
        {description && (
          <p className="mt-0.5 text-[12px] text-app-text-secondary">
            {description}
          </p>
        )}
      </div>
    </label>
  );
}

// ─── Main component ──────────────────────────────────────────────────────────

export default function AccountPreferences() {
  const { t } = useTranslation();
  const openDialog = useAppStore((s) => s.openDialog);
  const setOpenDialog = useAppStore((s) => s.setOpenDialog);
  const isOpen = openDialog === "account-prefs";

  const account = useAccountStore((s) => s.activeAccount());
  const quota = useAccountStore((s) => s.activeQuota());

  const [prefs, setPrefs] = useState<Preferences>(DEFAULT_PREFS);
  const [syncFolders, setSyncFolders] = useState<SyncFolder[]>([]);
  const [hostname, setHostname] = useState("Computer");
  const [photoQuality, setPhotoQuality] = useState<"original" | "storage_saver">("original");
  const [showQualityMenu, setShowQualityMenu] = useState(false);
  const [syncMode, setSyncMode] = useState<"stream" | "mirror">("stream");

  // Load preferences, hostname, and folders when dialog opens.
  useEffect(() => {
    if (!isOpen) return;
    invoke<string>("get_hostname")
      .then((name) => { if (name) setHostname(name); })
      .catch(() => {});
    invoke<Preferences>("get_preferences")
      .then((p) => {
        if (p) {
          setPrefs(p);
          if (p.vfs?.sync_mode) setSyncMode(p.vfs.sync_mode);
        }
      })
      .catch(() => {});
    invoke<SyncFolder[]>("get_sync_folders")
      .then((folders) => {
        if (folders) setSyncFolders(folders);
      })
      .catch(() => {});
  }, [isOpen]);

  // Apply language change when user selects a new one.
  useEffect(() => {
    const lang = prefs.general.language;
    if (lang === "follow_account") {
      invoke<string>("get_account_locale")
        .then((locale) => {
          if (locale) i18next.changeLanguage(locale);
          else i18next.changeLanguage("en");
        })
        .catch(() => i18next.changeLanguage("en"));
    } else {
      i18next.changeLanguage(lang);
    }
  }, [prefs.general.language]);

  const handleClose = useCallback(() => {
    // Persist current preferences.
    invoke("save_preferences", { prefs }).catch(() => {});
    applyThemeFromMode(prefs.general.appearance);
    setOpenDialog(null);
  }, [prefs, setOpenDialog]);

  const updateGeneral = (patch: Partial<GeneralPrefs>) =>
    setPrefs((p) => ({ ...p, general: { ...p.general, ...patch } }));

  const updateNetwork = (patch: Partial<NetworkPrefs>) =>
    setPrefs((p) => ({ ...p, network: { ...p.network, ...patch } }));

  const updateHotkeys = (patch: Partial<HotkeyPrefs>) =>
    setPrefs((p) => ({ ...p, hotkeys: { ...p.hotkeys, ...patch } }));

  const updateTelemetry = (patch: Partial<TelemetryPrefs>) =>
    setPrefs((p) => ({ ...p, telemetry: { ...p.telemetry, ...patch } }));

  const handleDisconnect = () => {
    if (!account) return;
    invoke("disconnect_account", { accountId: account.id }).catch(() => {});
  };

  const handleManageStorage = () => {
    invoke("open_url", { url: GOOGLE_STORAGE }).catch(
      () => {},
    );
  };

  const handleChangeMountPoint = async () => {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (selected && typeof selected === "string") {
        setPrefs((p) => ({
          ...p,
          vfs: { ...p.vfs, mount_point: selected },
        }));
      }
    } catch {
      // User cancelled.
    }
  };

  const handleResetMountPoint = () => {
    setPrefs((p) => ({ ...p, vfs: { ...p.vfs, mount_point: "~/GoogleDrive" } }));
  };

  const handleRemoveFolder = (id: string) => {
    setSyncFolders((f) => f.filter((folder) => folder.id !== id));
  };

  // Storage display.
  const limitGb =
    quota?.limit && quota.limit > 0
      ? (quota.limit / (1024 * 1024 * 1024)).toFixed(2)
      : DEFAULT_STORAGE_LIMIT_GB;
  const usageGb =
    quota && quota.usage > 0
      ? (quota.usage / (1024 * 1024 * 1024)).toFixed(2)
      : "0.00";

  const photosFolders = syncFolders.filter((f) => f.type === "photos");

  return (
    <Dialog open={isOpen} onClose={handleClose} className="w-[680px]">
      <DialogHeader onClose={handleClose}>{t("dialogs.account_prefs.title")}</DialogHeader>

      <DialogContent className="p-0">
        {/* ── Area 1: Account & Service Settings ──────────────────────────── */}
        <div className="space-y-8 p-6">
          {/* Account info row */}
          {account && (
            <div className="flex items-center gap-4 rounded-lg border border-app-border p-4">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center overflow-hidden rounded-full bg-app-accent text-sm font-medium text-app-surface">
                {account.photo_url ? (
                  <img
                    src={account.photo_url}
                    alt=""
                    className="h-full w-full object-cover"
                  />
                ) : (
                  <span>
                    {account.display_name?.charAt(0)?.toUpperCase() ??
                      account.email.charAt(0).toUpperCase()}
                  </span>
                )}
              </div>
              <div className="min-w-0 flex-1">
                <p className="truncate text-[14px] text-app-text-primary">
                  {account.email}
                </p>
                <p className="text-[12px] text-app-text-secondary">
                  {t("dialogs.account_prefs.using_storage", { used: usageGb, total: limitGb })}
                </p>
              </div>
              <div className="flex shrink-0 gap-3">
                <button
                  className="text-[12px] font-medium text-app-accent hover:underline"
                  onClick={handleDisconnect}
                >
                  {t("common.disconnect_account")}
                </button>
                <button
                  className="text-[12px] font-medium text-app-accent hover:underline"
                  onClick={handleManageStorage}
                >
                  {t("common.manage_storage")}
                </button>
              </div>
            </div>
          )}

          {/* Google Drive section */}
          <div>
            <SectionHeading
              icon={<Monitor className="h-5 w-5" />}
              title={t("common.google_drive")}
              subtitle={t("dialogs.preferences.drive_subtitle")}
            />

            <div className="ms-8 space-y-4">
              {/* Streaming location */}
              <div>
                <p className="mb-1 text-[13px] text-app-text-primary">
                  {t("dialogs.account_prefs.streaming_location")}
                </p>
                <div className="flex items-center gap-2">
                  <span className="rounded bg-app-subtle px-2 py-1 text-[12px] text-app-text-secondary">
                    {prefs.vfs.mount_point}
                  </span>
                  <button
                    className="text-[12px] font-medium text-app-accent hover:underline"
                    onClick={handleResetMountPoint}
                  >
                    {t("dialogs.account_prefs.reset_to_default")}
                  </button>
                  <button
                    className="text-[12px] font-medium text-app-accent hover:underline"
                    onClick={handleChangeMountPoint}
                  >
                    {t("dialogs.account_prefs.change")}
                  </button>
                </div>
              </div>

              {/* Real-time Presence */}
              <CheckRow
                label={t("dialogs.account_prefs.realtime_presence")}
                description={t("dialogs.account_prefs.realtime_presence_desc")}
                checked={true}
                onChange={() => {}}
              />
            </div>
          </div>

          {/* Google Photos section */}
          <div>
            <SectionHeading
              icon={<Image className="h-5 w-5" />}
              title={t("common.google_photos")}
            />

            <div className="ms-8 space-y-3">
              <div className="flex items-center gap-2">
                <p className="text-[13px] text-app-text-primary">
                  {t("dialogs.account_prefs.upload_size")}
                </p>
                <div className="relative">
                  <button
                    className="flex items-center gap-1 rounded border border-app-border px-3 py-1.5 text-[13px] text-app-text-primary transition-colors hover:bg-app-subtle"
                    onClick={() => setShowQualityMenu(!showQualityMenu)}
                  >
                    {photoQuality === "original"
                      ? t("common.original_quality")
                      : t("common.storage_saver")}
                    <ChevronDown className="h-3 w-3" />
                  </button>
                  {showQualityMenu && (
                    <div className="absolute start-0 top-full z-10 mt-1 w-44 rounded-lg border border-app-border bg-app-surface py-1 shadow-lg">
                      <button
                        className={`w-full px-4 py-2 text-start text-[13px] transition-colors hover:bg-app-subtle ${
                          photoQuality === "original"
                            ? "text-app-accent"
                            : "text-app-text-primary"
                        }`}
                        onClick={() => {
                          setPhotoQuality("original");
                          setShowQualityMenu(false);
                        }}
                      >
                        {t("common.original_quality")}
                      </button>
                      <button
                        className={`w-full px-4 py-2 text-start text-[13px] transition-colors hover:bg-app-subtle ${
                          photoQuality === "storage_saver"
                            ? "text-app-accent"
                            : "text-app-text-primary"
                        }`}
                        onClick={() => {
                          setPhotoQuality("storage_saver");
                          setShowQualityMenu(false);
                        }}
                      >
                        {t("common.storage_saver")}
                      </button>
                    </div>
                  )}
                </div>
              </div>

              {photosFolders.length > 0 && (
                <p className="text-[12px] text-app-text-secondary">
                  {t("dialogs.account_prefs.backing_up_count", { count: photosFolders.length })}
                </p>
              )}
            </div>
          </div>

          {/* Sync folders (combined list) */}
          {syncFolders.length > 0 && (
            <div>
              <SectionHeading
                icon={<FolderOpen className="h-5 w-5" />}
                title={t("dialogs.account_prefs.my_computer", { hostname })}
                subtitle={t("dialogs.account_prefs.folders_syncing")}
              />

              <div className="ms-8 rounded-lg border border-app-border">
                {syncFolders.map((folder, index) => (
                  <div
                    key={folder.id}
                    className={`flex items-center justify-between px-4 py-3 ${
                      index < syncFolders.length - 1
                        ? "border-b border-app-border"
                        : ""
                    }`}
                  >
                    <div className="flex items-center gap-3">
                      {folder.type === "drive" ? (
                        <HardDrive className="h-5 w-5 text-app-text-secondary" />
                      ) : (
                        <Image className="h-5 w-5 text-app-text-secondary" />
                      )}
                      <div>
                        <p className="text-[14px] text-app-text-primary">
                          {folder.name}
                        </p>
                        <p className="text-[12px] text-app-text-secondary">
                          {folder.path}
                          {folder.size != null &&
                            ` · ${formatSize(folder.size)}`}
                        </p>
                      </div>
                    </div>
                    <div className="flex items-center gap-2">
                      {folder.type === "photos" && (
                        <span className="rounded-full bg-app-hover px-2 py-0.5 text-[11px] text-app-text-secondary">
                          {photoQuality === "original"
                            ? t("common.original_quality")
                            : t("common.storage_saver")}
                        </span>
                      )}
                      <button
                        onClick={() => handleRemoveFolder(folder.id)}
                        className="flex h-8 w-8 items-center justify-center rounded-full text-app-text-secondary transition-colors hover:bg-app-hover"
                      >
                        <Trash2 className="h-4 w-4" />
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* ── Divider ────────────────────────────────────────────────────── */}
          <div className="border-t border-app-border" />

          {/* ── Area 2: App-wide General Settings ──────────────────────────── */}

          {/* Google Drive sync mode */}
          <div>
            <SectionHeading
              icon={<Cloud className="h-5 w-5" />}
              title={t("common.google_drive")}
              subtitle={t("dialogs.account_prefs.choose_storage")}
            />

            <div className="ms-8 space-y-3">
              <label className="flex cursor-pointer items-start gap-3">
                <input
                  type="radio"
                  name="syncMode"
                  checked={syncMode === "stream"}
                  onChange={() => {
                    setSyncMode("stream");
                    setPrefs((p) => ({ ...p, vfs: { ...p.vfs, sync_mode: "stream" } }));
                  }}
                  className="mt-1 h-4 w-4 accent-app-accent"
                />
                <div>
                  <p className="text-[13px] text-app-text-primary">
                    {t("dialogs.account_prefs.stream_files")}
                  </p>
                  <p className="text-[12px] text-app-text-secondary">
                    {t("dialogs.account_prefs.stream_desc")}
                  </p>
                </div>
              </label>
              <label className="flex cursor-pointer items-start gap-3">
                <input
                  type="radio"
                  name="syncMode"
                  checked={syncMode === "mirror"}
                  onChange={() => {
                    setSyncMode("mirror");
                    setPrefs((p) => ({ ...p, vfs: { ...p.vfs, sync_mode: "mirror" } }));
                  }}
                  className="mt-1 h-4 w-4 accent-app-accent"
                />
                <div>
                  <p className="text-[13px] text-app-text-primary">
                    {t("dialogs.account_prefs.mirror_files")}
                  </p>
                  <p className="text-[12px] text-app-text-secondary">
                    {t("dialogs.account_prefs.mirror_desc")}
                  </p>
                </div>
              </label>
              <div className="rounded-lg bg-status-active-bg dark:bg-[var(--color-status-syncing-bg-dark)] p-3">
                <p className="text-[12px] text-app-accent">
                  {t("dialogs.account_prefs.streaming_info")}
                </p>
              </div>
            </div>
          </div>

          {/* Launch on login */}
          <div className="ms-8">
            <CheckRow
              label={t("dialogs.account_prefs.launch_on_login")}
              description={t("dialogs.account_prefs.launch_on_login_desc")}
              checked={prefs.general.launch_on_login}
              onChange={(v) => updateGeneral({ launch_on_login: v })}
            />
          </div>

          {/* Proxy settings */}
          <div>
            <SectionHeading
              icon={<Cloud className="h-5 w-5" />}
              title={t("dialogs.account_prefs.proxy_settings")}
            />
            <div className="ms-8 space-y-2">
              <label className="flex cursor-pointer items-center gap-3">
                <input
                  type="radio"
                  name="proxy"
                  checked={prefs.network.proxy === "auto"}
                  onChange={() => updateNetwork({ proxy: "auto" })}
                  className="h-4 w-4 accent-app-accent"
                />
                <span className="text-[13px] text-app-text-primary">
                  {t("common.auto_detect")}
                </span>
              </label>
              <label className="flex cursor-pointer items-center gap-3">
                <input
                  type="radio"
                  name="proxy"
                  checked={prefs.network.proxy === "direct"}
                  onChange={() => updateNetwork({ proxy: "direct" })}
                  className="h-4 w-4 accent-app-accent"
                />
                <span className="text-[13px] text-app-text-primary">
                  {t("common.direct_connection")}
                </span>
              </label>
            </div>
          </div>

          {/* Bandwidth settings */}
          <div>
            <SectionHeading
              icon={<Cloud className="h-5 w-5" />}
              title={t("dialogs.account_prefs.bandwidth_settings")}
            />
            <div className="ms-8 space-y-3">
              <div>
                <CheckRow
                  label={t("dialogs.account_prefs.limit_download")}
                  checked={prefs.network.download_rate_limit > 0}
                  onChange={(v) =>
                    updateNetwork({
                      download_rate_limit: v ? DEFAULT_RATE_LIMIT_KBPS : 0,
                    })
                  }
                />
                {prefs.network.download_rate_limit > 0 && (
                  <div className="ms-7 mt-1 flex items-center gap-2">
                    <input
                      type="number"
                      value={prefs.network.download_rate_limit}
                      onChange={(e) =>
                        updateNetwork({
                          download_rate_limit: Math.max(
                            0,
                            parseInt(e.target.value) || 0,
                          ),
                        })
                      }
                      className="w-24 rounded border border-app-border bg-app-surface px-2 py-1 text-[13px] text-app-text-primary"
                    />
                    <span className="text-[12px] text-app-text-secondary">
                      {t("dialogs.account_prefs.kb_per_sec")}
                    </span>
                  </div>
                )}
              </div>
              <div>
                <CheckRow
                  label={t("dialogs.account_prefs.limit_upload")}
                  checked={prefs.network.upload_rate_limit > 0}
                  onChange={(v) =>
                    updateNetwork({
                      upload_rate_limit: v ? DEFAULT_RATE_LIMIT_KBPS : 0,
                    })
                  }
                />
                {prefs.network.upload_rate_limit > 0 && (
                  <div className="ms-7 mt-1 flex items-center gap-2">
                    <input
                      type="number"
                      value={prefs.network.upload_rate_limit}
                      onChange={(e) =>
                        updateNetwork({
                          upload_rate_limit: Math.max(
                            0,
                            parseInt(e.target.value) || 0,
                          ),
                        })
                      }
                      className="w-24 rounded border border-app-border bg-app-surface px-2 py-1 text-[13px] text-app-text-primary"
                    />
                    <span className="text-[12px] text-app-text-secondary">
                      {t("dialogs.account_prefs.kb_per_sec")}
                    </span>
                  </div>
                )}
              </div>
            </div>
          </div>

          {/* Hotkey */}
          <div>
            <SectionHeading
              icon={<Cloud className="h-5 w-5" />}
              title={t("dialogs.account_prefs.configure_hotkey")}
            />
            <div className="ms-8 space-y-2">
              <CheckRow
                label={t("dialogs.account_prefs.enable_hotkey")}
                checked={prefs.hotkeys.search_enabled}
                onChange={(v) => updateHotkeys({ search_enabled: v })}
              />
              {prefs.hotkeys.search_enabled && (
                <div className="ms-7 flex items-center gap-2">
                  <input
                    type="text"
                    value={prefs.hotkeys.search_key}
                    readOnly
                    className="w-40 rounded border border-app-border bg-app-subtle px-2 py-1 text-center text-[13px] text-app-text-primary"
                  />
                  <span className="text-[12px] text-app-text-secondary">
                    {t("dialogs.account_prefs.hotkey_hint")}
                  </span>
                </div>
              )}
            </div>
          </div>

          {/* Appearance */}
          <div>
            <SectionHeading
              icon={<Cloud className="h-5 w-5" />}
              title={t("dialogs.account_prefs.appearance")}
            />
            <div className="ms-8 space-y-2">
              {(
                [
                  ["light", t("dialogs.account_prefs.light")],
                  ["dark", t("dialogs.account_prefs.dark")],
                  ["follow_system", t("common.follow_system")],
                ] as const
              ).map(([value, label]) => (
                <label
                  key={value}
                  className="flex cursor-pointer items-center gap-3"
                >
                  <input
                    type="radio"
                    name="appearance"
                    checked={prefs.general.appearance === value}
                    onChange={() => updateGeneral({ appearance: value })}
                    className="h-4 w-4 accent-app-accent"
                  />
                  <span className="text-[13px] text-app-text-primary">
                    {label}
                  </span>
                </label>
              ))}
            </div>
          </div>

          {/* Language */}
          <div>
            <SectionHeading
              icon={<Cloud className="h-5 w-5" />}
              title={t("dialogs.account_prefs.language")}
            />
            <div className="ms-8">
              <select
                value={prefs.general.language}
                onChange={(e) => updateGeneral({ language: e.target.value })}
                className="rounded border border-app-border bg-app-surface px-3 py-1.5 text-[13px] text-app-text-primary"
              >
                {LANGUAGES.map((lang) => (
                  <option key={lang.value} value={lang.value}>
                    {"labelKey" in lang ? t((lang as { labelKey: string }).labelKey) : lang.label}
                  </option>
                ))}
              </select>
            </div>
          </div>

          {/* Telemetry */}
          <div className="ms-8">
            <CheckRow
              label={t("dialogs.account_prefs.telemetry")}
              description={t("dialogs.account_prefs.telemetry_desc")}
              checked={prefs.telemetry.auto_send_diagnostics}
              onChange={(v) =>
                updateTelemetry({ auto_send_diagnostics: v })
              }
            />
          </div>

          {/* Notification preferences */}
          <div className="ms-8">
            <CheckRow
              label={t("dialogs.account_prefs.prompt_backup")}
              checked={prefs.general.prompt_backup_devices}
              onChange={(v) =>
                updateGeneral({ prompt_backup_devices: v })
              }
            />
          </div>
        </div>
      </DialogContent>

      <DialogFooter>
        <Button variant="outline" onClick={handleClose}>
          {t("common.done")}
        </Button>
      </DialogFooter>
    </Dialog>
  );
}
