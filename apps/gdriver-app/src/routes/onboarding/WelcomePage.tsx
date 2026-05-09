import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import DriveLogo from "@/components/DriveLogo";
import { useAppStore } from "@/store/appStore";

function FeatureCard({
  icon,
  title,
  description,
}: {
  icon: React.ReactNode;
  title: string;
  description: string;
}) {
  return (
    <div className="flex flex-col items-center gap-2 rounded-xl p-4 text-center">
      <div className="mb-1 flex h-12 w-12 items-center justify-center rounded-full bg-status-active-bg text-app-accent dark:bg-[var(--color-status-syncing-bg-dark)]">
        {icon}
      </div>
      <h3 className="text-[13px] font-medium text-app-text-primary">
        {title}
      </h3>
      <p className="text-[11px] leading-relaxed text-app-text-secondary">
        {description}
      </p>
    </div>
  );
}

function ShieldIcon() {
  return (
    <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
    </svg>
  );
}

function AppIcon() {
  return (
    <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <rect x="2" y="3" width="20" height="14" rx="2" ry="2" />
      <line x1="8" y1="21" x2="16" y2="21" />
      <line x1="12" y1="17" x2="12" y2="21" />
    </svg>
  );
}

function SyncIcon() {
  return (
    <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="1 4 1 10 7 10" />
      <polyline points="23 20 23 14 17 14" />
      <path d="M21 10a9 9 0 0 0-16.9-4M3 14a9 9 0 0 0 16.9 4" />
    </svg>
  );
}

export default function WelcomePage() {
  const { t } = useTranslation();
  const nextStep = useAppStore((s) => s.nextStep);

  const features = [
    { icon: <ShieldIcon />, title: t("onboarding.welcome.feature_secure_title"), description: "" },
    { icon: <AppIcon />, title: t("onboarding.welcome.feature_app_title"), description: "" },
    { icon: <SyncIcon />, title: t("onboarding.welcome.feature_sync_title"), description: "" },
  ];

  return (
    <div className="flex flex-col items-center">
      {/* Logo + Title */}
      <div className="mb-3 flex items-center gap-3">
        <DriveLogo size={56} />
        <span className="text-[28px] font-normal text-app-text-primary">
          Drive
        </span>
      </div>

      <h1 className="mb-1 text-[26px] font-normal text-app-text-primary">
        {t("onboarding.welcome.title")}
      </h1>
      <p className="mb-8 text-[14px] text-app-text-secondary">
        {t("onboarding.welcome.subtitle")}
      </p>

      {/* Feature Cards */}
      <div className="mb-10 grid grid-cols-3 gap-2">
        {features.map((f) => (
          <FeatureCard key={f.title} {...f} />
        ))}
      </div>

      {/* Get started button */}
      <div className="flex w-full justify-end">
        <button
          onClick={nextStep}
          className="rounded-full bg-app-accent px-6 py-2.5 text-[14px] font-medium text-white transition-colors hover:bg-app-accent-hover focus:outline-none focus-visible:ring-2 focus-visible:ring-app-accent/50"
        >
          {t("onboarding.welcome.get_started")}
        </button>
      </div>

      {/* Top-right menu */}
      <div className="fixed end-3 top-3">
        <button
          onClick={() => {
            try { invoke("quit_app"); } catch { window.close(); }
          }}
          className="rounded-full p-1.5 text-app-text-secondary transition-colors hover:bg-app-hover"
          title={t("onboarding.welcome.quit_tooltip")}
        >
          <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
            <circle cx="12" cy="5" r="2" />
            <circle cx="12" cy="12" r="2" />
            <circle cx="12" cy="19" r="2" />
          </svg>
        </button>
      </div>
    </div>
  );
}
