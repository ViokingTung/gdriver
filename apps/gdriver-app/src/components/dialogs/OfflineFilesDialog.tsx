import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
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
import { HardDrive, FileText, ChevronDown, ChevronRight } from "lucide-react";

interface OfflineStats {
  offline_bytes: number;
  cache_bytes: number;
}

interface DriveStats {
  file_count: number;
  folder_count: number;
}

export default function OfflineFilesDialog() {
  const { t } = useTranslation();
  const openDialog = useAppStore((s) => s.openDialog);
  const setOpenDialog = useAppStore((s) => s.setOpenDialog);
  const isOpen = openDialog === "offline-files";

  const [showFiles, setShowFiles] = useState(false);
  const [isClearing, setIsClearing] = useState(false);
  const [offlineStats, setOfflineStats] = useState<OfflineStats>({
    offline_bytes: 0,
    cache_bytes: 0,
  });
  const [driveStats, setDriveStats] = useState<DriveStats>({
    file_count: 0,
    folder_count: 0,
  });
  const [accounts, setAccounts] = useState<{ email: string }[]>([]);

  useEffect(() => {
    if (!isOpen) return;

    invoke<OfflineStats>("offline.get_stats")
      .then((stats) => {
        if (stats) setOfflineStats(stats);
      })
      .catch(() => {});

    invoke<DriveStats>("system.get_drive_stats")
      .then((stats) => {
        if (stats) setDriveStats(stats);
      })
      .catch(() => {});

    invoke<{ email: string }[]>("auth.get_accounts")
      .then((accs) => {
        if (accs) setAccounts(accs);
      })
      .catch(() => {});
  }, [isOpen]);

  const handleClose = () => setOpenDialog(null);

  const handleClearOfflineFiles = async () => {
    setIsClearing(true);
    try {
      await invoke("offline.clear_cache");
      setOfflineStats({ offline_bytes: 0, cache_bytes: 0 });
    } catch {
      // ignore
    } finally {
      setIsClearing(false);
    }
  };

  const totalSize = offlineStats.offline_bytes + offlineStats.cache_bytes;
  const hasOfflineFiles = totalSize > 0;

  return (
    <Dialog open={isOpen} onClose={handleClose} className="w-[560px]">
      <DialogHeader onClose={handleClose}>
        <span className="text-[16px]">{t("dialogs.offline_files.title")}</span>
      </DialogHeader>

      <DialogContent className="p-6">
        {/* Description text */}
        <p className="mb-6 text-[13px] text-app-text-secondary">
          {t("dialogs.offline_files.description")}
        </p>

        {/* Clear offline files button */}
        <Button
          variant="outline"
          onClick={handleClearOfflineFiles}
          disabled={!hasOfflineFiles || isClearing}
          className="mb-6"
        >
          {isClearing
            ? t("dialogs.offline_files.clearing")
            : t("dialogs.offline_files.clear_offline")}
        </Button>

        {/* Account section */}
        <div className="rounded-lg border border-app-border">
          {/* Account header */}
          <div className="px-4 py-3">
            <p className="text-[14px] text-app-text-primary">
              {accounts[0]?.email ?? " "}
            </p>
          </div>

          {/* Google Drive folder */}
          <div className="border-t border-app-border">
            <div className="flex items-center justify-between px-4 py-3">
              <div className="flex items-center gap-3">
                <HardDrive className="h-5 w-5 text-app-text-secondary" />
                <div>
                  <p className="text-[14px] text-app-text-primary">
                    {t("common.google_drive")}
                  </p>
                  <p className="text-[12px] text-app-text-secondary">
                    {t("dialogs.offline_files.file_count", {
                      count: driveStats.file_count,
                      folders: driveStats.folder_count,
                    })}
                  </p>
                </div>
              </div>
            </div>

            {/* Offline files section */}
            <div className="border-t border-app-border">
              <button
                className="flex w-full items-center justify-between px-4 py-3 transition-colors hover:bg-app-subtle"
                onClick={() => setShowFiles(!showFiles)}
              >
                <div className="flex items-center gap-3">
                  <FileText className="h-5 w-5 text-app-text-secondary" />
                  <span className="text-[14px] text-app-text-primary">
                    {t("dialogs.offline_files.offline_files")}
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <span className="text-[13px] text-app-text-secondary">
                    {formatSize(totalSize, { allowZero: true })}
                  </span>
                  {showFiles ? (
                    <ChevronDown className="h-4 w-4 text-app-text-secondary" />
                  ) : (
                    <ChevronRight className="h-4 w-4 text-app-text-secondary" />
                  )}
                </div>
              </button>

              {/* File list - expanded view */}
              {showFiles && (
                <div className="border-t border-app-border">
                  <div className="px-4 py-3 ps-12">
                    <p className="text-[13px] text-app-text-secondary">
                      {t("dialogs.offline_files.no_offline_files")}
                    </p>
                  </div>
                </div>
              )}
            </div>
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
