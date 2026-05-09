import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { useAppStore, SUGGESTED_PHOTOS_FOLDERS } from "@/store/appStore";
import { formatSize } from "@/lib/formatSize";

function PhotosIcon() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" className="text-app-text-secondary">
      <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
      <circle cx="8.5" cy="8.5" r="1.5" />
      <polyline points="21 15 16 10 5 21" />
    </svg>
  );
}

export default function PhotosPage() {
  const { t } = useTranslation();
  const { pendingPhotosFolders, togglePhotosFolder, addPhotosFolder, prevStep, nextStep } =
    useAppStore();

  const isSelected = (id: string) => pendingPhotosFolders.some((f) => f.id === id);

  return (
    <div className="flex flex-col">
      <h1 className="mb-1 text-[22px] font-normal text-app-text-primary">
        {t("onboarding.photos.title")}
      </h1>
      <p className="mb-6 text-[13px] text-app-text-secondary">
        {t("onboarding.photos.subtitle")}
      </p>

      {/* Suggested folders */}
      <div className="mb-2">
        <p className="mb-2 text-[11px] font-medium uppercase tracking-wide text-app-text-muted">
          {t("onboarding.photos.suggested_folders")}
        </p>
        <div className="space-y-1">
          {SUGGESTED_PHOTOS_FOLDERS.map((folder) => (
            <label
              key={folder.id}
              className={`flex cursor-pointer items-center gap-3 rounded-lg border px-3 py-2.5 transition-colors ${
                isSelected(folder.id)
                  ? "border-app-accent bg-status-active-bg dark:bg-[var(--color-status-syncing-bg-dark)]/30"
                  : "border-app-border bg-app-surface hover:bg-app-bg-secondary dark:bg-transparent"
              }`}
            >
              <input
                type="checkbox"
                checked={isSelected(folder.id)}
                onChange={() => togglePhotosFolder(folder.id)}
                className="h-4 w-4 rounded accent-app-accent"
              />
              <PhotosIcon />
              <div className="flex-1">
                <p className="text-[13px] font-medium text-app-text-primary">
                  {folder.name}
                </p>
                <p className="text-[11px] text-app-text-muted">
                  {folder.path}
                </p>
              </div>
              <span className="rounded-full bg-status-active-bg px-2 py-0.5 text-[10px] font-medium text-app-accent">
                {t("common.suggested_folder")}
              </span>
            </label>
          ))}
        </div>
      </div>

      {/* Custom added folders */}
      {pendingPhotosFolders.filter((f) => !f.isSuggested).length > 0 && (
        <div className="mb-2">
          <div className="space-y-1">
            {pendingPhotosFolders
              .filter((f) => !f.isSuggested)
              .map((folder) => (
                <div
                  key={folder.id}
                  className="flex items-center gap-3 rounded-lg border border-app-accent bg-status-active-bg px-3 py-2.5 dark:bg-[var(--color-status-syncing-bg-dark)]/30"
                >
                  <input
                    type="checkbox"
                    checked={true}
                    onChange={() => togglePhotosFolder(folder.id)}
                    className="h-4 w-4 rounded accent-app-accent"
                  />
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

      {/* Add folder button */}
      <button
        onClick={async () => {
          try {
            const selected = await open({ directory: true, multiple: false });
            if (selected && typeof selected === "string") {
              const name = selected.split("/").pop() ?? selected;
              const id = `photos-custom-${Date.now()}`;
              addPhotosFolder({
                id,
                name,
                path: selected,
                type: "photos",
              });
            }
          } catch {
            // User cancelled
          }
        }}
        className="mb-6 flex items-center gap-2 text-[13px] font-medium text-app-accent hover:text-app-accent-hover"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
          <line x1="12" y1="5" x2="12" y2="19" />
          <line x1="5" y1="12" x2="19" y2="12" />
        </svg>
        {t("common.add_folder")}
      </button>

      {/* Bottom info bar */}
      <div className="mb-6 rounded-lg bg-status-active-bg px-4 py-3 dark:bg-[var(--color-status-syncing-bg-dark)]/30">
        <p className="text-[12px] leading-relaxed text-app-accent">
          {t("onboarding.photos.info_bar")}
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
