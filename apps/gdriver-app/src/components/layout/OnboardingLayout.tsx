import { type ReactNode } from "react";
import { useTranslation } from "react-i18next";

interface OnboardingLayoutProps {
  step: number;
  children: ReactNode;
}

function indicatorState(
  currentStep: number,
  indicatorNumber: number,
): "completed" | "active" | "inactive" {
  // Map onboarding step to which indicator step is active
  let activeIndicator = 0;
  if (currentStep >= 2 && currentStep <= 3) activeIndicator = 1;
  else if (currentStep >= 4 && currentStep <= 5) activeIndicator = 2;
  else if (currentStep >= 6) activeIndicator = 3;

  if (indicatorNumber < activeIndicator) return "completed";
  if (indicatorNumber === activeIndicator) return "active";
  return "inactive";
}

function showIndicator(step: number): boolean {
  return step >= 2 && step <= 6;
}

export default function OnboardingLayout({ step, children }: OnboardingLayoutProps) {
  const { t } = useTranslation();
  const visible = showIndicator(step);

  const steps = [
    { number: 1, label: t("onboarding.steps.sync_drive"), sublabel: t("onboarding.steps.optional") },
    { number: 2, label: t("onboarding.steps.backup_photos"), sublabel: t("onboarding.steps.optional") },
    { number: 3, label: t("onboarding.steps.see_files") },
  ];

  return (
    <div className="flex h-screen w-full bg-app-bg-primary">
      {visible && (
        <aside className="flex w-[240px] shrink-0 flex-col border-e border-app-border bg-app-bg-secondary px-6 pt-12">
          <nav className="flex flex-col gap-0">
            {steps.map((s, i) => {
              const state = indicatorState(step, s.number);
              const isLast = i === steps.length - 1;
              return (
                <div key={s.number} className="flex gap-3">
                  <div className="flex flex-col items-center">
                    <div
                      className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-full text-sm font-medium transition-colors ${
                        state === "completed"
                          ? "bg-app-accent text-white"
                          : state === "active"
                            ? "bg-app-accent text-white"
                            : "border-2 border-app-border bg-app-surface text-app-text-secondary dark:bg-transparent"
                      }`}
                    >
                      {state === "completed" ? (
                        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
                          <polyline points="20 6 9 17 4 12" />
                        </svg>
                      ) : (
                        s.number
                      )}
                    </div>
                    {!isLast && (
                      <div
                        className={`my-0.5 w-0.5 flex-1 min-h-[32px] ${
                          state === "completed" ? "bg-app-accent" : "bg-app-border"
                        }`}
                      />
                    )}
                  </div>
                  <div className="pb-6">
                    <p
                      className={`text-[13px] leading-tight ${
                        state === "active"
                          ? "font-medium text-app-accent"
                          : "text-app-text-secondary"
                      }`}
                    >
                      {s.label}
                    </p>
                    {s.sublabel && (
                      <p className="text-[11px] text-app-text-muted">
                        {s.sublabel}
                      </p>
                    )}
                  </div>
                </div>
              );
            })}
          </nav>
        </aside>
      )}
      <main className="flex flex-1 flex-col items-center justify-center px-10">
        <div className="w-full max-w-[540px]">{children}</div>
      </main>
    </div>
  );
}
