import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { useAppStore, type FolderItem } from "@/store/appStore";
import { formatSize } from "@/lib/formatSize";

const SUGGESTED_FOLDERS: FolderItem[] = [
  { id: "desktop", name: "Desktop", path: "~/Desktop", type: "drive", isSuggested: true },
  { id: "documents", name: "Documents", path: "~/Documents", type: "drive", isSuggested: true },
  { id: "downloads", name: "Downloads", path: "~/Downloads", type: "drive", isSuggested: true },
];

function FolderIcon() {
  return (
    <svg width="20" height="20" viewBox="0 0 24 24" fill="currentColor" stroke="none" className="text-app-text-secondary">
      <path d="M10 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z" />
    </svg>
  );
}

export default function SyncFolderPage() {
  const { t } = useTranslation();
  const { pendingDriveFolders, toggleDriveFolder, addDriveFolder, nextStep, skipToStep } =
    useAppStore();

  const isSelected = (id: string) => pendingDriveFolders.some((f) => f.id === id);

  const hasSelection = pendingDriveFolders.length > 0;

  const totalSize = pendingDriveFolders.reduce((sum, f) => sum + (f.size || 0), 0);

  return (
    <div className="flex flex-col">
      <h1 className="mb-1 text-[22px] font-normal text-app-text-primary">
        {t("onboarding.sync_folder.title")}
      </h1>
      <p className="mb-6 text-[13px] text-app-text-secondary">
        {t("onboarding.sync_folder.subtitle")}
      </p>

      {/* Suggested folders */}
      <div className="mb-2">
        <p className="mb-2 text-[11px] font-medium uppercase tracking-wide text-app-text-muted">
          {t("onboarding.sync_folder.suggested_folders")}
        </p>
        <div className="space-y-1">
          {SUGGESTED_FOLDERS.map((folder) => (
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
                onChange={() => toggleDriveFolder(folder.id)}
                className="h-4 w-4 rounded accent-app-accent"
              />
              <FolderIcon />
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
              {isSelected(folder.id) && folder.size && (
                <span className="text-[12px] text-app-text-secondary">
                  {formatSize(folder.size)}
                </span>
              )}
            </label>
          ))}
        </div>
      </div>

      {/* Custom added folders */}
      {pendingDriveFolders.filter((f) => !f.isSuggested).length > 0 && (
        <div className="mb-2">
          <p className="mb-2 text-[11px] font-medium uppercase tracking-wide text-app-text-muted">
            {t("onboarding.sync_folder.added_folders")}
          </p>
          <div className="space-y-1">
            {pendingDriveFolders
              .filter((f) => !f.isSuggested)
              .map((folder) => (
                <div
                  key={folder.id}
                  className="flex items-center gap-3 rounded-lg border border-app-accent bg-status-active-bg px-3 py-2.5 dark:bg-[var(--color-status-syncing-bg-dark)]/30"
                >
                  <input
                    type="checkbox"
                    checked={true}
                    onChange={() => toggleDriveFolder(folder.id)}
                    className="h-4 w-4 rounded accent-app-accent"
                  />
                  <FolderIcon />
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
          const selected = await open({
            directory: true,
            multiple: false,
            title: t("common.add_folder"),
          });
          if (selected) {
            const path = selected as string;
            const name = path.split("/").pop() || path;
            const id = `custom-${Date.now()}`;
            addDriveFolder({
              id,
              name,
              path,
              type: "drive",
            });
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
          {t("onboarding.sync_folder.info_bar")}
        </p>
      </div>

      {/* Navigation */}
      <div className="flex items-center justify-between">
        <button
          onClick={() => skipToStep(4)}
          className="rounded-full px-5 py-2 text-[13px] font-medium text-app-accent transition-colors hover:bg-status-active-bg"
        >
          {t("common.skip")}
        </button>
        <div className="flex items-center gap-2">
          {hasSelection && (
            <span className="text-[12px] text-app-text-secondary">
              {totalSize > 0
                ? t("common.folders_selected_with_size", { count: pendingDriveFolders.length, size: formatSize(totalSize) })
                : t("common.folders_selected", { count: pendingDriveFolders.length })}
            </span>
          )}
          <button
            onClick={nextStep}
            disabled={!hasSelection}
            className={`rounded-full px-6 py-2.5 text-[14px] font-medium transition-colors ${
              hasSelection
                ? "bg-app-accent text-white hover:bg-app-accent-hover"
                : "cursor-not-allowed bg-app-border text-app-text-muted"
            }`}
          >
            {t("common.next")}
          </button>
        </div>
      </div>
    </div>
  );
}
