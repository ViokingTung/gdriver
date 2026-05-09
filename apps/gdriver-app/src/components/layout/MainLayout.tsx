import TopBar from "@/components/layout/TopBar";
import Sidebar from "@/components/layout/Sidebar";
import HomePage from "@/routes/main/HomePage";
import SyncActivityPage from "@/routes/main/SyncActivityPage";
import NotificationsPage from "@/routes/main/NotificationsPage";
import PreferencesDialog from "@/components/dialogs/PreferencesDialog";
import OfflineFilesDialog from "@/components/dialogs/OfflineFilesDialog";
import ErrorListDialog from "@/components/dialogs/ErrorListDialog";
import AboutDialog from "@/components/dialogs/AboutDialog";
import FeedbackDialog from "@/components/dialogs/FeedbackDialog";
import AccountPreferences from "@/components/account/AccountPreferences";
import { useAppStore } from "@/store/appStore";

const PAGES: Record<string, React.ComponentType> = {
  home: HomePage,
  sync: SyncActivityPage,
  notifications: NotificationsPage,
};

export default function MainLayout() {
  const currentPage = useAppStore((s) => s.currentPage);
  const PageComponent = PAGES[currentPage] ?? HomePage;

  return (
    <div className="flex h-screen flex-col bg-app-bg-primary">
      <TopBar />
      <div className="flex flex-1 min-h-0">
        <Sidebar />
        <main className="flex-1 overflow-y-auto p-6">
          <PageComponent />
        </main>
      </div>
      <PreferencesDialog />
      <OfflineFilesDialog />
      <ErrorListDialog />
      <AboutDialog />
      <FeedbackDialog />
      <AccountPreferences />
    </div>
  );
}
