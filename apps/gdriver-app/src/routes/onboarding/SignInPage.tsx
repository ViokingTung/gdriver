import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useAppStore } from "@/store/appStore";
import { GOOGLE_ACCOUNTS_SIGNUP } from "@/lib/urls";
import DriveLogo from "@/components/DriveLogo";

export default function SignInPage() {
  const { t } = useTranslation();
  const nextStep = useAppStore((s) => s.nextStep);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const pollingRef = useRef(false);

  // If an account already exists, skip sign-in entirely.
  useEffect(() => {
    if (!isTauri()) return;
    invoke<Array<{ id: string }>>("get_accounts").then((accounts) => {
      if (accounts && accounts.length > 0) {
        nextStep();
      }
    }).catch(() => { /* ignore */ });
  }, [nextStep]);

  // Listen for the push event as a fast path during active OAuth.
  useEffect(() => {
    if (!isTauri()) return;
    const unlistenPromise = listen("onboarding:oauth-complete", () => {
      nextStep();
    });
    return () => {
      unlistenPromise.then((fn) => fn());
    };
  }, [nextStep]);

  const handleSignIn = async () => {
    setLoading(true);
    setError(null);
    try {
      if (!isTauri()) {
        setError("Sign-in requires the desktop app. Run `pnpm tauri dev` to test.");
        setLoading(false);
        return;
      }

      // Record existing account count before starting OAuth.
      let accountCountBefore = 0;
      try {
        const existing = await invoke<Array<{ id: string }>>("get_accounts");
        accountCountBefore = existing?.length ?? 0;
      } catch {
        // ignore
      }

      const authUrl = await invoke<string>("start_oauth_flow");
      await invoke("open_url", { url: authUrl });

      // Poll for a NEW account (count must increase).
      pollingRef.current = true;
      const poll = async () => {
        while (pollingRef.current) {
          await new Promise((r) => setTimeout(r, 2000));
          if (!pollingRef.current) break;
          try {
            const accounts = await invoke<Array<{ id: string }>>("get_accounts");
            if (accounts && accounts.length > accountCountBefore) {
              pollingRef.current = false;
              nextStep();
              return;
            }
          } catch {
            // ignore
          }
        }
      };
      poll();
    } catch (e) {
      setError(typeof e === "string" ? e : String(e));
      setLoading(false);
      pollingRef.current = false;
    }
  };

  // Stop polling when component unmounts.
  useEffect(() => {
    return () => {
      pollingRef.current = false;
    };
  }, []);

  return (
    <div className="flex flex-col items-center">
      {/* Logo */}
      <div className="mb-6">
        <DriveLogo size={64} />
      </div>

      <h1 className="mb-2 text-center text-[24px] font-normal text-app-text-primary">
        {t("onboarding.signin.title")}
      </h1>
      <p className="mb-8 text-center text-[14px] leading-relaxed text-app-text-secondary">
        {t("onboarding.signin.subtitle")}
      </p>

      {/* Sign in button */}
      <button
        onClick={handleSignIn}
        disabled={loading}
        className="mb-4 flex items-center gap-3 rounded-full bg-app-accent px-8 py-2.5 text-[14px] font-medium text-white transition-colors hover:bg-app-accent-hover focus:outline-none focus-visible:ring-2 focus-visible:ring-app-accent/50 disabled:opacity-50"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
          <path d="M12.545 10.239v3.821h5.445c-.712 2.315-2.647 3.972-5.445 3.972a5.979 5.979 0 110-11.957 5.58 5.58 0 013.817 1.466l2.826-2.826A9.535 9.535 0 0012.545 2C7.021 2 2.543 6.477 2.543 12s4.478 10 10.002 10c8.396 0 10.89-7.85 9.745-12.239H12.545z" />
        </svg>
        {loading ? t("onboarding.signin.signing_in") : t("onboarding.signin.sign_in")}
      </button>

      {/* Error message */}
      {error && (
        <p className="mb-4 max-w-sm text-center text-[13px] text-red-500">
          {error}
        </p>
      )}

      {/* Create account link */}
      <button
        onClick={() => {
          window.open(GOOGLE_ACCOUNTS_SIGNUP, "_blank");
        }}
        className="text-[13px] font-medium text-app-accent hover:text-app-accent-hover"
      >
        {t("onboarding.signin.create_account")}
      </button>

      {/* Illustration placeholder */}
      <div className="mt-10 flex items-center justify-center">
        <div className="flex h-40 w-80 items-center justify-center rounded-lg border border-dashed border-app-border bg-app-bg-secondary">
          <div className="text-center text-[12px] text-app-text-muted">
            <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1" className="mx-auto mb-2 opacity-40">
              <rect x="3" y="3" width="18" height="18" rx="2" />
              <circle cx="8.5" cy="8.5" r="1.5" />
              <path d="M21 15l-5-5L5 21" />
            </svg>
            {t("onboarding.signin.illustration_alt")}
          </div>
        </div>
      </div>
    </div>
  );
}
