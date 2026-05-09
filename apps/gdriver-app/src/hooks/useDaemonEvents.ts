import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import i18next from "i18next";
import { useQueryClient } from "@tanstack/react-query";
import { useSyncStore } from "@/store/syncStore";
import { useAccountStore } from "@/store/accountStore";
import type { SyncStatusPayload, SyncItem } from "@/types/sync";
import type { Account, StorageQuota } from "@/types/account";

/**
 * Subscribe to all daemon push events and pipe them into Zustand stores.
 *
 * Call this once from the app root (e.g. `App.tsx`) when the user reaches
 * the main phase.  The hook cleans up listeners on unmount.
 */
export function useDaemonEvents() {
  const queryClient = useQueryClient();

  useEffect(() => {
    const unlisteners: Array<() => void> = [];

    // Hydrate the sync store with the daemon's current status on mount.
    invoke<SyncStatusPayload>("get_sync_status")
      .then((payload) => {
        if (payload) useSyncStore.getState().setStatus(payload);
      })
      .catch(() => {
        // Daemon not ready yet — the first push event will update us.
      });

    // Hydrate accounts from the daemon.
    invoke<Account[]>("get_accounts")
      .then((accounts) => {
        if (accounts) {
          useAccountStore.getState().setAccounts(accounts);
          // Fetch quota for each account.
          for (const acct of accounts) {
            invoke<StorageQuota>("get_storage_quota", { accountId: acct.id })
              .then((quota) => {
                if (quota) useAccountStore.getState().setQuota(acct.id, quota);
              })
              .catch(() => {});
          }
          // Apply account locale if user hasn't set a manual preference.
          const manualLang = localStorage.getItem("i18nextLng");
          if (!manualLang || manualLang === "follow_account") {
            invoke<string>("get_account_locale")
              .then((locale) => {
                if (locale) i18next.changeLanguage(locale);
              })
              .catch(() => {});
          }
        }
      })
      .catch(() => {
        // Daemon not ready — the account:changed event will hydrate us.
      });

    // Listen for real-time status pushes.
    listen<SyncStatusPayload>("sync:status-changed", ({ payload }) => {
      useSyncStore.getState().setStatus(payload);
    }).then((unlisten) => {
      unlisteners.push(unlisten);
    });

    // Listen for per-file sync updates → invalidate recent items and activity queries.
    listen<SyncItem>("sync:item-updated", () => {
      queryClient.invalidateQueries({ queryKey: ["recent-sync-items"] });
      queryClient.invalidateQueries({ queryKey: ["sync-activity"] });
    }).then((unlisten) => {
      unlisteners.push(unlisten);
    });

    // Listen for new notifications → invalidate notification queries.
    listen("notification:new", () => {
      queryClient.invalidateQueries({ queryKey: ["notifications"] });
      queryClient.invalidateQueries({ queryKey: ["notifications-summary"] });
    }).then((unlisten) => {
      unlisteners.push(unlisten);
    });

    // Listen for account list changes (add / disconnect).
    listen<{ accounts: Account[] }>("account:changed", ({ payload }) => {
      useAccountStore.getState().setAccounts(payload.accounts);
      // Fetch quota for any newly added accounts.
      for (const acct of payload.accounts) {
        if (!useAccountStore.getState().quotas[acct.id]) {
          invoke<StorageQuota>("get_storage_quota", { accountId: acct.id })
            .then((quota) => {
              if (quota) useAccountStore.getState().setQuota(acct.id, quota);
            })
            .catch(() => {});
        }
      }
    }).then((unlisten) => {
      unlisteners.push(unlisten);
    });

    // Listen for storage quota updates.
    listen<{ accountId: string; quota: StorageQuota }>(
      "account:quota-updated",
      ({ payload }) => {
        useAccountStore.getState().setQuota(payload.accountId, payload.quota);
      },
    ).then((unlisten) => {
      unlisteners.push(unlisten);
    });

    return () => {
      unlisteners.forEach((fn) => fn());
    };
  }, [queryClient]);
}
