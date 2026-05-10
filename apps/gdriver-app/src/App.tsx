import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "@/store/appStore";
import { useDaemonEvents } from "@/hooks/useDaemonEvents";
import { useTheme } from "@/hooks/useTheme";
import OnboardingLayout from "@/components/layout/OnboardingLayout";
import MainLayout from "@/components/layout/MainLayout";
import WelcomePage from "@/routes/onboarding/WelcomePage";
import SignInPage from "@/routes/onboarding/SignInPage";
import SyncFolderPage from "@/routes/onboarding/SyncFolderPage";
import SyncFolderConfirmPage from "@/routes/onboarding/SyncFolderConfirmPage";
import PhotosPage from "@/routes/onboarding/PhotosPage";
import FolderSummaryPage from "@/routes/onboarding/FolderSummaryPage";
import FileManagerPage from "@/routes/onboarding/FileManagerPage";
import OfflinePage from "@/routes/onboarding/OfflinePage";

const ONBOARDING_PAGES: Record<number, React.ComponentType> = {
  0: WelcomePage,
  1: SignInPage,
  2: SyncFolderPage,
  3: SyncFolderConfirmPage,
  4: PhotosPage,
  5: FolderSummaryPage,
  6: FileManagerPage,
  7: OfflinePage,
};

function App() {
  useTheme();
  const phase = useAppStore((s) => s.phase);
  const step = useAppStore((s) => s.onboardingStep);
  const setPhase = useAppStore((s) => s.setPhase);

  // Skip onboarding if an account is already connected.
  useEffect(() => {
    invoke<unknown[]>("get_accounts")
      .then((accounts) => {
        if (accounts && accounts.length > 0) {
          setPhase("main");
        }
      })
      .catch(() => {});
  }, [setPhase]);

  // Subscribe to daemon push events once we reach the main phase.
  if (phase === "main") {
    return <MainWithEvents />;
  }

  const PageComponent = ONBOARDING_PAGES[step] ?? WelcomePage;

  return (
    <OnboardingLayout step={step}>
      <PageComponent />
    </OnboardingLayout>
  );
}

/** Wrapper that activates daemon event listeners before rendering MainLayout. */
function MainWithEvents() {
  useDaemonEvents();
  return <MainLayout />;
}

export default App;
