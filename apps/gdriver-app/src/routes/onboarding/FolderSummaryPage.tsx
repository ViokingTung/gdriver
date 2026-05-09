import { useTranslation } from "react-i18next";
import { useAppStore } from "@/store/appStore";
import { formatSize } from "@/lib/formatSize";

function DriveIcon() {
  return (
    <svg width="20" height="20" viewBox="0 0 84 71" fill="none" xmlns="http://www.w3.org/2000/svg">
      <path d="M55.9721 46.9762L41.9989 70.9995L28.0256 46.9762H55.9721Z" fill="#FBBC05" />
      <path d="M70.0003 23.5838L55.9998 47.0002L41.9993 23.5838H70.0003Z" fill="#34A853" />
      <path d="M55.973 46.9763L41.9991 70.9997L14.0281 23.023H41.9991L55.973 46.9763Z" fill="#4285F4" />
      <path d="M41.9998 23.0234L27.9996 46.4398L14 23.0234L27.9998 -0.000244141L41.9998 23.0234Z" fill="#EA4335" />
      <path d="M14.0002 23.0239L28.0004 -0.000366211L42.0006 23.0239L28.0004 46.4402L14.0002 23.0239Z" fill="#1967D2" />
    </svg>
  );
}

function PhotosIcon() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" fill="none">
      <circle cx="12" cy="12" r="10" fill="#FBBC05" />
      <path d="M12 6l2.5 5 5.5.8-4 3.9.9 5.5-4.9-2.6L7.1 21.2l.9-5.5-4-3.9 5.5-.8L12 6z" fill="#EA4335" />
    </svg>
  );
}

export default function FolderSummaryPage() {
  const { t } = useTranslation();
  const { pendingDriveFolders, pendingPhotosFolders, prevStep, nextStep } = useAppStore();

  const allFolders = [...pendingDriveFolders, ...pendingPhotosFolders];
  const totalSize = allFolders.reduce((sum, f) => sum + (f.size || 0), 0);

  const hasDrive = pendingDriveFolders.length > 0;
  const hasPhotos = pendingPhotosFolders.length > 0;

  let title = t("onboarding.folder_summary.title_drive");
  if (hasDrive && hasPhotos) {
    title = t("onboarding.folder_summary.title_both");
  } else if (hasPhotos) {
    title = t("onboarding.folder_summary.title_photos");
  }

  return (
    <div className="flex flex-col">
      <h1 className="mb-1 text-[22px] font-normal text-app-text-primary">
        {title}
      </h1>
      <p className="mb-6 text-[13px] text-app-text-secondary">
        {t("onboarding.folder_summary.subtitle")}
      </p>

      {/* Drive folders */}
      {hasDrive && (
        <div className="mb-3">
          <p className="mb-2 text-[11px] font-medium uppercase tracking-wide text-app-text-muted">
            {t("onboarding.folder_summary.syncing_to_drive")}
          </p>
          <div className="space-y-1">
            {pendingDriveFolders.map((folder) => (
              <div
                key={folder.id}
                className="flex items-center gap-3 rounded-lg border border-app-border px-3 py-2.5"
              >
                <DriveIcon />
                <div className="flex-1">
                  <p className="text-[13px] font-medium text-app-text-primary">
                    {folder.name}
                  </p>
                  <p className="text-[11px] text-app-text-muted">
                    {folder.path}
                  </p>
                </div>
                {folder.size && (
                  <span className="text-[12px] text-app-text-secondary">
                    {formatSize(folder.size)}
                  </span>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Photos folders */}
      {hasPhotos && (
        <div className="mb-3">
          <p className="mb-2 text-[11px] font-medium uppercase tracking-wide text-app-text-muted">
            {t("onboarding.folder_summary.backing_up_to_photos")}
          </p>
          <div className="space-y-1">
            {pendingPhotosFolders.map((folder) => (
              <div
                key={folder.id}
                className="flex items-center gap-3 rounded-lg border border-app-border px-3 py-2.5"
              >
                <PhotosIcon />
                <div className="flex-1">
                  <p className="text-[13px] font-medium text-app-text-primary">
                    {folder.name}
                  </p>
                  <p className="text-[11px] text-app-text-muted">
                    {folder.path}
                  </p>
                </div>
                {folder.size && (
                  <span className="text-[12px] text-app-text-secondary">
                    {formatSize(folder.size)}
                  </span>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Summary */}
      <p className="mb-6 text-[13px] text-app-text-secondary">
        {totalSize > 0
          ? t("common.folders_selected_with_size", { count: allFolders.length, size: formatSize(totalSize) })
          : t("common.folders_selected", { count: allFolders.length })}
      </p>

      {/* Bottom info bar */}
      <div className="mb-6 rounded-lg bg-status-active-bg px-4 py-3 dark:bg-[var(--color-status-syncing-bg-dark)]/30">
        <p className="text-[12px] leading-relaxed text-app-accent">
          {t("onboarding.folder_summary.info_bar")}
        </p>
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
