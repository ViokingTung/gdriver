import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useAppStore, type MainPage } from "@/store/appStore";
import { Folder, Home, RefreshCw, Bell } from "lucide-react";

interface NavItem {
  id: MainPage;
  labelKey: string;
  icon: typeof Home;
}

const NAV_ITEMS: NavItem[] = [
  { id: "home", labelKey: "sidebar.home", icon: Home },
  { id: "sync", labelKey: "sidebar.sync_activity", icon: RefreshCw },
  { id: "notifications", labelKey: "sidebar.notifications", icon: Bell },
];

export default function Sidebar() {
  const { t } = useTranslation();
  const currentPage = useAppStore((s) => s.currentPage);
  const setCurrentPage = useAppStore((s) => s.setCurrentPage);

  return (
    <aside className="flex w-[248px] shrink-0 flex-col border-e border-app-border bg-app-bg-secondary">
      {/* Open Drive folder */}
      <div className="px-3 pt-3">
        <button
          onClick={() => {
            try { invoke("open_drive_folder"); } catch { /* M8 */ }
          }}
          className="flex w-full items-center gap-3 rounded-full bg-app-surface px-4 py-2.5 text-[13px] font-medium text-app-text-primary shadow-sm transition-colors hover:bg-app-subtle"
        >
          <Folder className="h-5 w-5 text-app-text-secondary" />
          {t("sidebar.open_drive_folder")}
        </button>
      </div>

      {/* Navigation */}
      <nav className="mt-2 flex-1 px-3">
        {NAV_ITEMS.map((item) => {
          const isActive = currentPage === item.id;
          const Icon = item.icon;
          return (
            <button
              key={item.id}
              onClick={() => setCurrentPage(item.id)}
              className={`flex w-full items-center gap-3 rounded-full px-4 py-2.5 text-[13px] font-medium transition-colors ${
                isActive
                  ? "bg-status-active-bg text-app-accent dark:bg-[var(--color-status-syncing-bg-dark)]"
                  : "text-app-text-primary hover:bg-app-hover"
              }`}
            >
              <Icon className={`h-5 w-5 ${isActive ? "text-app-accent" : "text-app-text-secondary"}`} />
              {t(item.labelKey)}
            </button>
          );
        })}
      </nav>
    </aside>
  );
}
