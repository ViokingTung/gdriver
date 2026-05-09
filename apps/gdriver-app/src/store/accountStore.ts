import { create } from "zustand";
import type { Account, StorageQuota } from "@/types/account";

interface AccountState {
  accounts: Account[];
  activeAccountId: string | null;
  quotas: Record<string, StorageQuota>;

  setAccounts: (accounts: Account[]) => void;
  setQuota: (accountId: string, quota: StorageQuota) => void;
  setActiveAccount: (accountId: string) => void;

  /** Get the currently active account, falling back to the first account. */
  activeAccount: () => Account | null;
  /** Get the quota for the active account. */
  activeQuota: () => StorageQuota | null;
}

export const useAccountStore = create<AccountState>((set, get) => ({
  accounts: [],
  activeAccountId: null,
  quotas: {},

  setAccounts: (accounts) => {
    const { activeAccountId } = get();
    // If the active account was removed, reset to the first account.
    if (activeAccountId && !accounts.find((a) => a.id === activeAccountId)) {
      set({ accounts, activeAccountId: accounts[0]?.id ?? null });
    } else {
      // If no active account is set, default to the first one.
      set({
        accounts,
        activeAccountId: activeAccountId ?? accounts[0]?.id ?? null,
      });
    }
  },

  setQuota: (accountId, quota) =>
    set((s) => ({ quotas: { ...s.quotas, [accountId]: quota } })),

  setActiveAccount: (accountId) => set({ activeAccountId: accountId }),

  activeAccount: () => {
    const { accounts, activeAccountId } = get();
    return accounts.find((a) => a.id === activeAccountId) ?? accounts[0] ?? null;
  },

  activeQuota: () => {
    const { quotas, activeAccountId, accounts } = get();
    const id = activeAccountId ?? accounts[0]?.id;
    return id ? quotas[id] ?? null : null;
  },
}));
