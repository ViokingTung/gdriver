import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useTranslation } from "react-i18next";
import { useAppStore } from "@/store/appStore";
import { formatSize } from "@/lib/formatSize";
import {
  Dialog,
  DialogHeader,
  DialogContent,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import {
  FolderOpen,
  Trash2,
  HardDrive,
  Image,
  Monitor,
  ChevronDown,
} from "lucide-react";

interface SyncFolder {
  id: string;
  name: string;
  path: string;
  type: "drive" | "photos";
  size?: number;
}

export default function PreferencesDialog() {
  const { t } = useTranslation();
  const openDialog = useAppStore((s) => s.openDialog);
  const setOpenDialog = useAppStore((s) => s.setOpenDialog);
  const isOpen = openDialog === "preferences";

  const [activeTab, setActiveTab] = useState<"computer" | "drive">("computer");
  const [syncFolders, setSyncFolders] = useState<SyncFolder[]>([]);
  const [hostname, setHostname] = useState("Computer");

  const [syncMode, setSyncMode] = useState<"stream" | "mirror">("stream");

  // Load hostname, sync folders, and sync mode from backend when dialog opens
  useEffect(() => {
    if (!isOpen) return;
    invoke<string>("get_hostname")
      .then((name) => { if (name) setHostname(name); })
      .catch(() => {});
    invoke<SyncFolder[]>("get_sync_folders")
      .then((folders) => {
        console.log("[get_sync_folders] result:", folders);
        if (folders) setSyncFolders(folders);
      })
      .catch((e) => {
        console.error("[get_sync_folders] failed:", e);
      });
    invoke<any>("get_preferences")
      .then((prefs) => {
        if (prefs?.vfs?.sync_mode) setSyncMode(prefs.vfs.sync_mode);
      })
      .catch(() => {});
  }, [isOpen]);
  const [showQualityMenu, setShowQualityMenu] = useState<string | null>(null);
  const [photoQuality, setPhotoQuality] = useState<"original" | "storage-saver">("original");

  const handleClose = () => setOpenDialog(null);

  const handleAddFolder = async () => {
    let selected: string | null = null;
    try {
      const result = await open({ directory: true, multiple: false });
      if (result && typeof result === "string") {
        selected = result;
      }
    } catch {
      return; // 用户取消选择
    }
    if (!selected) return;
    try {
      console.log("[add_sync_folder] calling with path:", selected);
      const saved = await invoke<SyncFolder>("add_sync_folder", {
        path: selected,
        folderType: "drive",
      });
      console.log("[add_sync_folder] result:", saved);
      if (saved) setSyncFolders([...syncFolders, saved]);
    } catch (e) {
      console.error("[add_sync_folder] failed:", e);
    }
  };

  const handleRemoveFolder = async (id: string) => {
    setSyncFolders(syncFolders.filter((f) => f.id !== id));
    try {
      await invoke("remove_sync_folder", { folderId: id });
    } catch {
      // Ignore — folder already removed from UI state
    }
  };

  const handleOpenFileManager = async () => {
    try {
      await invoke("open_drive_folder");
    } catch (e) {
      console.error("open_drive_folder failed:", e);
    }
  };

  const handleChangeSyncMode = async (mode: "stream" | "mirror") => {
    setSyncMode(mode);
    try {
      await invoke("set_sync_mode", { mode });
    } catch (e) {
      console.error("set_sync_mode failed:", e);
    }
  };

  const driveFolders = syncFolders.filter((f) => f.type === "drive");
  const photosFolders = syncFolders.filter((f) => f.type === "photos");

  return (
    <Dialog open={isOpen} onClose={handleClose} className="w-[680px]">
      <DialogHeader onClose={handleClose}>{t("dialogs.preferences.title")}</DialogHeader>

      {/* Tabs */}
      <div className="flex border-b border-app-border px-6">
        <button
          className={`flex items-center gap-2 border-b-2 px-4 py-3 text-[14px] font-medium transition-colors ${
            activeTab === "computer"
              ? "border-app-accent text-app-accent"
              : "border-transparent text-app-text-secondary hover:text-app-text-primary"
          }`}
          onClick={() => setActiveTab("computer")}
        >
          <Monitor className="h-5 w-5" />
          {t("dialogs.preferences.tab_computer", { hostname })}
        </button>
        <button
          className={`flex items-center gap-2 border-b-2 px-4 py-3 text-[14px] font-medium transition-colors ${
            activeTab === "drive"
              ? "border-app-accent text-app-accent"
              : "border-transparent text-app-text-secondary hover:text-app-text-primary"
          }`}
          onClick={() => setActiveTab("drive")}
        >
          <HardDrive className="h-5 w-5" />
          {t("dialogs.preferences.tab_drive")}
        </button>
      </div>

      <DialogContent className="p-0">
        {activeTab === "computer" ? (
          <div className="p-6">
            {/* Add folder button */}
            <Button
              variant="outline"
              onClick={handleAddFolder}
              className="mb-6 gap-2"
            >
              <FolderOpen className="h-4 w-4" />
              {t("dialogs.preferences.add_folder")}
            </Button>

            {/* Google Drive section */}
            <div className="mb-6">
              <h3 className="mb-2 text-[16px] font-medium text-app-text-primary">
                {t("dialogs.preferences.drive_section")}
              </h3>
              <p className="mb-4 text-[13px] text-app-text-secondary">
                {t("dialogs.preferences.syncing_from_folders", { count: driveFolders.length })}
              </p>
              <div className="rounded-lg border border-app-border">
                {driveFolders.map((folder, index) => (
                  <div
                    key={folder.id}
                    className={`flex items-center justify-between px-4 py-3 ${
                      index < driveFolders.length - 1
                        ? "border-b border-app-border"
                        : ""
                    }`}
                  >
                    <div className="flex items-center gap-3">
                      <HardDrive className="h-5 w-5 text-app-text-secondary" />
                      <div>
                        <p className="text-[14px] text-app-text-primary">
                          {folder.name}
                        </p>
                        <p className="text-[12px] text-app-text-secondary">
                          {folder.path} &middot; {formatSize(folder.size || 0, { precision: 0 })}
                        </p>
                      </div>
                    </div>
                    <button
                      onClick={() => handleRemoveFolder(folder.id)}
                      className="flex h-8 w-8 items-center justify-center rounded-full text-app-text-secondary transition-colors hover:bg-app-hover"
                    >
                      <Trash2 className="h-4 w-4" />
                    </button>
                  </div>
                ))}
              </div>
            </div>

            {/* Google Photos section */}
            <div>
              <h3 className="mb-2 text-[16px] font-medium text-app-text-primary">
                {t("dialogs.preferences.photos_section")}
              </h3>
              <p className="mb-4 text-[13px] text-app-text-secondary">
                {t("dialogs.preferences.backing_up_folders", { count: photosFolders.length })}
                {" "}
                <button
                  className="text-app-accent hover:underline"
                  onClick={() => setShowQualityMenu(showQualityMenu ? null : "quality")}
                >
                  {photoQuality === "original" ? t("common.original_quality") : t("common.storage_saver")}
                  <ChevronDown className="ms-1 inline h-3 w-3" />
                </button>
              </p>
              <div className="rounded-lg border border-app-border">
                {photosFolders.map((folder, index) => (
                  <div
                    key={folder.id}
                    className={`flex items-center justify-between px-4 py-3 ${
                      index < photosFolders.length - 1
                        ? "border-b border-app-border"
                        : ""
                    }`}
                  >
                    <div className="flex items-center gap-3">
                      <Image className="h-5 w-5 text-app-text-secondary" />
                      <div>
                        <p className="text-[14px] text-app-text-primary">
                          {folder.name}
                        </p>
                        <p className="text-[12px] text-app-text-secondary">
                          {folder.path} &middot; {formatSize(folder.size || 0, { precision: 0 })}
                        </p>
                      </div>
                    </div>
                    <button
                      onClick={() => handleRemoveFolder(folder.id)}
                      className="flex h-8 w-8 items-center justify-center rounded-full text-app-text-secondary transition-colors hover:bg-app-hover"
                    >
                      <Trash2 className="h-4 w-4" />
                    </button>
                  </div>
                ))}
              </div>

              {/* Quality menu dropdown */}
              {showQualityMenu === "quality" && (
                <div className="relative mt-2">
                  <div className="absolute end-0 z-10 w-48 rounded-lg border border-app-border bg-app-surface py-1 shadow-lg">
                    <button
                      className={`w-full px-4 py-2 text-start text-[13px] transition-colors hover:bg-app-subtle ${
                        photoQuality === "original"
                          ? "text-app-accent"
                          : "text-app-text-primary"
                      }`}
                      onClick={() => {
                        setPhotoQuality("original");
                        setShowQualityMenu(null);
                      }}
                    >
                      {t("common.original_quality")}
                    </button>
                    <button
                      className={`w-full px-4 py-2 text-start text-[13px] transition-colors hover:bg-app-subtle ${
                        photoQuality === "storage-saver"
                          ? "text-app-accent"
                          : "text-app-text-primary"
                      }`}
                      onClick={() => {
                        setPhotoQuality("storage-saver");
                        setShowQualityMenu(null);
                      }}
                    >
                      {t("common.storage_saver")}
                    </button>
                  </div>
                </div>
              )}
            </div>
          </div>
        ) : (
          <div className="p-6">
            {/* Open in File Manager */}
            <button
              onClick={handleOpenFileManager}
              className="mb-6 flex items-center gap-2 text-[14px] text-app-accent hover:underline"
            >
              <FolderOpen className="h-4 w-4" />
              {t("dialogs.preferences.open_in_file_manager")}
            </button>

            {/* Sync mode section */}
            <div className="mb-6">
              <h3 className="mb-4 text-[16px] font-medium text-app-text-primary">
                {t("dialogs.preferences.my_drive")}
              </h3>
              <p className="mb-4 text-[13px] text-app-text-secondary">
                {t("dialogs.preferences.choose_storage")}
              </p>

              <div className="space-y-4">
                <label className="flex items-start gap-3 cursor-pointer">
                  <input
                    type="radio"
                    name="syncMode"
                    checked={syncMode === "stream"}
                    onChange={() => handleChangeSyncMode("stream")}
                    className="mt-1 h-4 w-4 accent-app-accent"
                  />
                  <div>
                    <p className="text-[14px] text-app-text-primary">
                      {t("dialogs.preferences.stream_files")}
                    </p>
                    <p className="text-[12px] text-app-text-secondary">
                      {t("dialogs.preferences.stream_desc")}
                    </p>
                  </div>
                </label>

                <label className="flex items-start gap-3 cursor-pointer">
                  <input
                    type="radio"
                    name="syncMode"
                    checked={syncMode === "mirror"}
                    onChange={() => handleChangeSyncMode("mirror")}
                    className="mt-1 h-4 w-4 accent-app-accent"
                  />
                  <div>
                    <p className="text-[14px] text-app-text-primary">
                      {t("dialogs.preferences.mirror_files")}
                    </p>
                    <p className="text-[12px] text-app-text-secondary">
                      {t("dialogs.preferences.mirror_desc")}
                    </p>
                  </div>
                </label>
              </div>
            </div>

            {/* Info bar */}
            <div className="rounded-lg bg-status-active-bg p-4 dark:bg-[var(--color-status-syncing-bg-dark)]">
              <p className="text-[13px] text-app-accent">
                {t("dialogs.preferences.streaming_info")}
              </p>
            </div>
          </div>
        )}
      </DialogContent>

      <DialogFooter>
        <Button variant="outline" onClick={handleClose}>
          {t("common.done")}
        </Button>
      </DialogFooter>
    </Dialog>
  );
}
