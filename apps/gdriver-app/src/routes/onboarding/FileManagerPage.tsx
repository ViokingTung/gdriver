import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { useAppStore } from "@/store/appStore";
import { GOOGLE_DRIVE_WEB } from "@/lib/urls";
import { Loader2 } from "lucide-react";

interface DriveStats {
  file_count: number;
  folder_count: number;
}

function getFileManagerName(): string {
  // Platform detection will be wired in a later milestone.
  return "File Manager";
}

export default function FileManagerPage() {
  const { t } = useTranslation();
  const { prevStep, nextStep } = useAppStore();
  const fmName = getFileManagerName();

  const { data: stats, isLoading } = useQuery<DriveStats>({
    queryKey: ["drive-stats"],
    queryFn: () => invoke<DriveStats>("get_drive_stats"),
  });

  return (
    <div className="flex flex-col">
      <h1 className="mb-3 text-[22px] font-normal text-app-text-primary">
        {t("onboarding.file_manager.title", { fmName })}
      </h1>
      <p className="mb-8 text-[13px] leading-relaxed text-app-text-secondary">
        {t("onboarding.file_manager.subtitle", { fmName })}
      </p>

      {/* Stats */}
      <div className="mb-6 flex items-center gap-2">
        <svg width="20" height="20" viewBox="0 0 84 71" fill="none" xmlns="http://www.w3.org/2000/svg" className="shrink-0">
          <path d="M55.9721 46.9762L41.9989 70.9995L28.0256 46.9762H55.9721Z" fill="#FBBC05" />
          <path d="M70.0003 23.5838L55.9998 47.0002L41.9993 23.5838H70.0003Z" fill="#34A853" />
          <path d="M55.973 46.9763L41.9991 70.9997L14.0281 23.023H41.9991L55.973 46.9763Z" fill="#4285F4" />
          <path d="M41.9998 23.0234L27.9996 46.4398L14 23.0234L27.9998 -0.000244141L41.9998 23.0234Z" fill="#EA4335" />
          <path d="M14.0002 23.0239L28.0004 -0.000366211L42.0006 23.0239L28.0004 46.4402L14.0002 23.0239Z" fill="#1967D2" />
        </svg>
        {isLoading ? (
          <Loader2 className="h-4 w-4 animate-spin text-app-text-secondary" />
        ) : (() => {
          const fc = stats?.file_count ?? 0;
          const foc = stats?.folder_count ?? 0;
          let text: string;
          if (fc === 0 && foc === 0) {
            text = t("onboarding.file_manager.no_files");
          } else {
            const files = fc > 0 ? t("onboarding.file_manager.file", { count: fc }) : "";
            const folders = foc > 0 ? t("onboarding.file_manager.folder", { count: foc }) : "";
            text = t("onboarding.file_manager.files_and_folders", { files, folders });
          }
          return <span className="text-[13px] text-app-text-secondary">{text}</span>;
        })()}
      </div>

      {/* Open file manager link */}
      <button
        onClick={() => {
          invoke("open_url", { url: GOOGLE_DRIVE_WEB }).catch(() => {});
        }}
        className="mb-8 flex items-center gap-2 text-[13px] font-medium text-app-accent hover:text-app-accent-hover"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" />
        </svg>
        {t("onboarding.file_manager.open_fm", { fmName })}
      </button>

      {/* Illustration placeholder */}
      <div className="mb-8 flex justify-center">
        <div className="flex h-44 w-72 items-center justify-center rounded-lg border border-dashed border-app-border bg-app-bg-secondary">
          <div className="text-center text-[12px] text-app-text-muted">
            <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1" className="mx-auto mb-2 opacity-40">
              <rect x="2" y="3" width="20" height="18" rx="2" />
              <rect x="6" y="7" width="12" height="9" rx="1" />
              <line x1="8" y1="3" x2="8" y2="7" />
              <line x1="16" y1="3" x2="16" y2="7" />
            </svg>
            {t("onboarding.file_manager.fm_sidebar", { fmName })}
          </div>
        </div>
      </div>

      {/* Navigation */}
      <div className="flex items-center justify-between">
        <button
          onClick={prevStep}
          className="rounded-full px-5 py-2 text-[13px] font-medium text-app-accent transition-colors hover:bg-status-active-bg dark:hover:bg-[var(--color-status-syncing-bg-dark)]/30"
        >
          {t("common.back")}
        </button>
        <button
          onClick={nextStep}
          className="rounded-full bg-app-accent px-6 py-2.5 text-[14px] font-medium text-white transition-colors hover:bg-app-accent-hover"
        >
          {t("common.next")}
        </button>
      </div>
    </div>
  );
}
