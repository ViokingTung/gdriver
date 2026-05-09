import { describe, it, expect, beforeEach } from "vitest";
import { useAccountStore } from "@/store/accountStore";
import type { Account, StorageQuota } from "@/types/account";

function makeAccount(overrides: Partial<Account> = {}): Account {
  return {
    id: "acct-1",
    email: "user@example.com",
    display_name: "Test User",
    photo_url: null,
    locale: "en",
    created_at: 1000,
    last_used_at: 2000,
    ...overrides,
  };
}

function makeQuota(overrides: Partial<StorageQuota> = {}): StorageQuota {
  return {
    limit: 15_000_000_000,
    usage: 5_000_000_000,
    usage_in_drive: 4_000_000_000,
    usage_in_drive_trash: 1_000_000_000,
    ...overrides,
  };
}

beforeEach(() => {
  useAccountStore.setState({
    accounts: [],
    activeAccountId: null,
    quotas: {},
  });
});

// ── Initial state ──────────────────────────────────────────────────────────

describe("initial state", () => {
  it("has empty accounts", () => {
    expect(useAccountStore.getState().accounts).toEqual([]);
  });

  it("has no active account", () => {
    expect(useAccountStore.getState().activeAccountId).toBeNull();
  });

  it("has empty quotas", () => {
    expect(useAccountStore.getState().quotas).toEqual({});
  });
});

// ── setAccounts ────────────────────────────────────────────────────────────

describe("setAccounts", () => {
  it("sets accounts and defaults active to first", () => {
    const a1 = makeAccount({ id: "a1" });
    const a2 = makeAccount({ id: "a2" });
    useAccountStore.getState().setAccounts([a1, a2]);
    const s = useAccountStore.getState();
    expect(s.accounts).toHaveLength(2);
    expect(s.activeAccountId).toBe("a1");
  });

  it("keeps existing activeAccountId if still present", () => {
    const a1 = makeAccount({ id: "a1" });
    const a2 = makeAccount({ id: "a2" });
    useAccountStore.setState({ activeAccountId: "a2" });
    useAccountStore.getState().setAccounts([a1, a2]);
    expect(useAccountStore.getState().activeAccountId).toBe("a2");
  });

  it("resets activeAccountId if removed from list", () => {
    const a1 = makeAccount({ id: "a1" });
    useAccountStore.setState({ activeAccountId: "a2" });
    useAccountStore.getState().setAccounts([a1]);
    expect(useAccountStore.getState().activeAccountId).toBe("a1");
  });

  it("sets activeAccountId to null if all accounts removed", () => {
    useAccountStore.setState({ activeAccountId: "a1" });
    useAccountStore.getState().setAccounts([]);
    expect(useAccountStore.getState().activeAccountId).toBeNull();
  });
});

// ── setActiveAccount ───────────────────────────────────────────────────────

describe("setActiveAccount", () => {
  it("sets the active account id", () => {
    useAccountStore.getState().setActiveAccount("acct-2");
    expect(useAccountStore.getState().activeAccountId).toBe("acct-2");
  });

  it("overwrites previous active account", () => {
    useAccountStore.getState().setActiveAccount("first");
    useAccountStore.getState().setActiveAccount("second");
    expect(useAccountStore.getState().activeAccountId).toBe("second");
  });
});

// ── setQuota ───────────────────────────────────────────────────────────────

describe("setQuota", () => {
  it("sets quota for an account", () => {
    const q = makeQuota();
    useAccountStore.getState().setQuota("acct-1", q);
    expect(useAccountStore.getState().quotas["acct-1"]).toEqual(q);
  });

  it("updates existing quota", () => {
    const q1 = makeQuota({ usage: 100 });
    const q2 = makeQuota({ usage: 200 });
    useAccountStore.getState().setQuota("acct-1", q1);
    useAccountStore.getState().setQuota("acct-1", q2);
    expect(useAccountStore.getState().quotas["acct-1"]!.usage).toBe(200);
  });

  it("stores quotas for multiple accounts", () => {
    useAccountStore.getState().setQuota("a", makeQuota({ usage: 1 }));
    useAccountStore.getState().setQuota("b", makeQuota({ usage: 2 }));
    const q = useAccountStore.getState().quotas;
    expect(Object.keys(q)).toHaveLength(2);
    expect(q["a"]!.usage).toBe(1);
    expect(q["b"]!.usage).toBe(2);
  });
});

// ── activeAccount selector ─────────────────────────────────────────────────

describe("activeAccount", () => {
  it("returns the active account", () => {
    const a = makeAccount({ id: "a1" });
    useAccountStore.setState({
      accounts: [a],
      activeAccountId: "a1",
    });
    expect(useAccountStore.getState().activeAccount()).toEqual(a);
  });

  it("falls back to first account when activeAccountId is null", () => {
    const a = makeAccount({ id: "a1" });
    useAccountStore.setState({ accounts: [a], activeAccountId: null });
    expect(useAccountStore.getState().activeAccount()).toEqual(a);
  });

  it("falls back to first account when activeAccountId not found", () => {
    const a = makeAccount({ id: "a1" });
    useAccountStore.setState({
      accounts: [a],
      activeAccountId: "missing",
    });
    expect(useAccountStore.getState().activeAccount()).toEqual(a);
  });

  it("returns null when no accounts exist", () => {
    expect(useAccountStore.getState().activeAccount()).toBeNull();
  });
});

// ── activeQuota selector ───────────────────────────────────────────────────

describe("activeQuota", () => {
  it("returns quota for the active account", () => {
    const q = makeQuota();
    useAccountStore.setState({
      accounts: [makeAccount({ id: "a1" })],
      activeAccountId: "a1",
      quotas: { a1: q },
    });
    expect(useAccountStore.getState().activeQuota()).toEqual(q);
  });

  it("falls back to first account's quota when activeAccountId is null", () => {
    const q = makeQuota();
    useAccountStore.setState({
      accounts: [makeAccount({ id: "a1" })],
      activeAccountId: null,
      quotas: { a1: q },
    });
    expect(useAccountStore.getState().activeQuota()).toEqual(q);
  });

  it("returns null when no quota exists for the account", () => {
    useAccountStore.setState({
      accounts: [makeAccount({ id: "a1" })],
      activeAccountId: "a1",
      quotas: {},
    });
    expect(useAccountStore.getState().activeQuota()).toBeNull();
  });

  it("returns null when no accounts exist", () => {
    expect(useAccountStore.getState().activeQuota()).toBeNull();
  });

  it("reads quota when account id is derived from first account", () => {
    const q = makeQuota();
    useAccountStore.setState({
      accounts: [makeAccount({ id: "first" })],
      activeAccountId: null,
      quotas: { first: q },
    });
    expect(useAccountStore.getState().activeQuota()).toEqual(q);
  });
});
