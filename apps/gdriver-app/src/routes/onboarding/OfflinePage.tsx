import { useTranslation } from "react-i18next";
import { useAppStore } from "@/store/appStore";
import { GOOGLE_DRIVE_OFFLINE_HELP } from "@/lib/urls";

export default function OfflinePage() {
  const { t } = useTranslation();
  const { prevStep, completeOnboarding } = useAppStore();

  return (
    <div className="flex flex-col items-center text-center">
      <h1 className="mb-3 text-[22px] font-normal text-app-text-primary">
        {t("onboarding.offline.title")}
      </h1>
      <p className="mb-10 max-w-md text-[13px] leading-relaxed text-app-text-secondary">
        {t("onboarding.offline.subtitle")}
      </p>

      {/* Step illustrations */}
      <div className="mb-8 flex items-center gap-6">
        {/* Step 1 */}
        <div className="flex flex-col items-center gap-3">
          <div className="flex h-28 w-44 items-center justify-center rounded-lg border border-dashed border-app-border bg-app-bg-secondary">
            <div className="text-center text-[11px] text-app-text-muted">
              <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1" className="mx-auto mb-1 opacity-40">
                <rect x="3" y="3" width="18" height="18" rx="2" />
                <line x1="3" y1="9" x2="21" y2="9" />
                <line x1="9" y1="21" x2="9" y2="9" />
              </svg>
              {t("onboarding.offline.step1_illustration")}
            </div>
          </div>
          <div className="flex h-6 w-6 items-center justify-center rounded-full bg-status-active-bg text-[12px] font-medium text-app-accent dark:bg-[var(--color-status-syncing-bg-dark)]">
            1
          </div>
          <p className="text-[12px] text-app-text-secondary">
            {t("onboarding.offline.step1_label")}
          </p>
        </div>

        {/* Arrow */}
        <svg width="24" height="24" viewBox="0 0 24 24" fill="none" className="stroke-app-border mt-[-20px]" strokeWidth="2">
          <line x1="5" y1="12" x2="19" y2="12" />
          <polyline points="12 5 19 12 12 19" />
        </svg>

        {/* Step 2 */}
        <div className="flex flex-col items-center gap-3">
          <div className="flex h-28 w-44 items-center justify-center rounded-lg border border-dashed border-app-border bg-app-bg-secondary">
            <div className="text-center text-[11px] text-app-text-muted">
              <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1" className="mx-auto mb-1 opacity-40">
                <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
                <polyline points="9 12 11 14 15 10" />
              </svg>
              {t("onboarding.offline.step2_illustration")}
            </div>
          </div>
          <div className="flex h-6 w-6 items-center justify-center rounded-full bg-status-active-bg text-[12px] font-medium text-app-accent dark:bg-[var(--color-status-syncing-bg-dark)]">
            2
          </div>
          <p className="text-[12px] text-app-text-secondary">
            {t("onboarding.offline.step2_label")}
          </p>
        </div>
      </div>

      {/* Learn more link */}
      <a
        href={GOOGLE_DRIVE_OFFLINE_HELP}
        target="_blank"
        rel="noreferrer"
        className="mb-8 text-[13px] font-medium text-app-accent hover:text-app-accent-hover"
      >
        {t("common.learn_more")}
      </a>

      {/* Navigation */}
      <div className="flex w-full items-center justify-between">
        <button
          onClick={prevStep}
          className="rounded-full px-5 py-2 text-[13px] font-medium text-app-accent transition-colors hover:bg-status-active-bg dark:hover:bg-[var(--color-status-syncing-bg-dark)]/30"
        >
          {t("common.back")}
        </button>
        <button
          onClick={completeOnboarding}
          className="rounded-full bg-app-accent px-6 py-2.5 text-[14px] font-medium text-white transition-colors hover:bg-app-accent-hover"
        >
          {t("onboarding.offline.open_drive")}
        </button>
      </div>
    </div>
  );
}
