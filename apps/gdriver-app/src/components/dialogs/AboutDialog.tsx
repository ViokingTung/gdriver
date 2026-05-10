import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useAppStore } from "@/store/appStore";
import {
  Dialog,
  DialogHeader,
  DialogContent,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import DriveLogo from "@/components/DriveLogo";
import { GOOGLE_PRIVACY, GOOGLE_TERMS, GITHUB_REPO, GITHUB_LICENSE } from "@/lib/urls";

export default function AboutDialog() {
  const { t } = useTranslation();
  const openDialog = useAppStore((s) => s.openDialog);
  const setOpenDialog = useAppStore((s) => s.setOpenDialog);
  const isOpen = openDialog === "about";

  const [version, setVersion] = useState<string>("...");
  const [platform, setPlatform] = useState<string>("...");

  useEffect(() => {
    if (isOpen) {
      // 获取应用版本
      invoke<string>("get_app_version")
        .then((v) => setVersion(v))
        .catch(() => {});

      // 获取平台信息
      invoke<string>("get_platform")
        .then((p) => setPlatform(p))
        .catch(() => {});
    }
  }, [isOpen]);

  const handleClose = () => setOpenDialog(null);

  const handleOpenUrl = (url: string) => {
    try {
      invoke("open_url", { url });
    } catch {
      // 忽略错误
    }
  };

  return (
    <Dialog open={isOpen} onClose={handleClose} className="w-[400px]">
      <DialogHeader onClose={handleClose}>
        <span className="text-[16px]">{t("dialogs.about.title")}</span>
      </DialogHeader>

      <DialogContent className="flex flex-col items-center py-8">
        {/* Google Drive Logo */}
        <div className="mb-4">
          <DriveLogo size={64} />
        </div>

        {/* App name */}
        <h2 className="mb-2 text-[20px] font-medium text-app-text-primary">
          {t("common.google_drive")}
        </h2>

        {/* Version */}
        <p className="mb-4 text-[13px] text-app-text-secondary">
          {t("dialogs.about.version", { version, platform })}
        </p>

        {/* Copyright - commented out for now */}
        {/* <p className="mb-6 text-[12px] text-app-text-secondary">
          {t("dialogs.about.copyright", { year: currentYear })}
        </p> */}

        {/* Links */}
        <div className="flex items-center gap-2 text-[12px]">
          <button
            onClick={() => handleOpenUrl(GITHUB_REPO)}
            className="text-app-accent hover:underline"
          >
            GitHub
          </button>
          <span className="text-app-text-secondary">|</span>
          <button
            onClick={() => handleOpenUrl(GITHUB_LICENSE)}
            className="text-app-accent hover:underline"
          >
            {t("dialogs.about.open_source")}
          </button>
          <span className="text-app-text-secondary">|</span>
          <button
            onClick={() => handleOpenUrl(GOOGLE_TERMS)}
            className="text-app-accent hover:underline"
          >
            {t("dialogs.about.terms")}
          </button>
          <span className="text-app-text-secondary">|</span>
          <button
            onClick={() =>
              handleOpenUrl(GOOGLE_PRIVACY)
            }
            className="text-app-accent hover:underline"
          >
            {t("dialogs.about.privacy")}
          </button>
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
