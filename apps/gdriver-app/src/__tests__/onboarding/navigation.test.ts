import { describe, it, expect, beforeEach } from "vitest";
import { useAppStore } from "@/store/appStore";
import type { FolderItem } from "@/store/appStore";

/**
 * Onboarding step layout:
 *   0 = Welcome
 *   1 = SignIn
 *   2 = SyncFolder (pick folders)
 *   3 = SyncFolderConfirm (confirmation page)
 *   4 = Photos
 *   5 = FileManager
 *   6 = FolderSummary
 *   7 = Offline
 *
 * The skip logic: step 3 (confirmation) is skipped when no drive folders
 * are selected. This applies both forward (nextStep) and backward (prevStep).
 */

function driveFolder(id: string): FolderItem {
  return { id, name: id, path: `~/${id}`, type: "drive" };
}

beforeEach(() => {
  useAppStore.setState({
    phase: "onboarding",
    onboardingStep: 0,
    pendingDriveFolders: [],
    pendingPhotosFolders: [],
    currentPage: "home",
    openDialog: null,
  });
});

// ── Full forward walk ──────────────────────────────────────────────────────

describe("forward navigation", () => {
  it("walks through all steps normally when drive folders are selected", () => {
    useAppStore.getState().toggleDriveFolder("desktop");
    const steps: number[] = [];
    for (let i = 0; i < 8; i++) {
      steps.push(useAppStore.getState().onboardingStep);
      useAppStore.getState().nextStep();
    }
    // With drive folders: 0→1→2→3→4→5→6→7→(step 8)
    expect(steps).toEqual([0, 1, 2, 3, 4, 5, 6, 7]);
  });

  it("skips confirmation (step 3) when no drive folders are selected", () => {
    const steps: number[] = [];
    for (let i = 0; i < 8; i++) {
      steps.push(useAppStore.getState().onboardingStep);
      useAppStore.getState().nextStep();
    }
    // Without drive folders: 0→1→2→4→5→6→7→(step 8)
    expect(steps).toEqual([0, 1, 2, 4, 5, 6, 7, 8]);
  });
});

// ── Full backward walk ─────────────────────────────────────────────────────

describe("backward navigation", () => {
  it("walks backwards through all steps normally when drive folders were selected", () => {
    useAppStore.getState().toggleDriveFolder("desktop");
    useAppStore.setState({ onboardingStep: 7 });
    const steps: number[] = [];
    for (let i = 0; i < 8; i++) {
      steps.push(useAppStore.getState().onboardingStep);
      useAppStore.getState().prevStep();
    }
    // With drive folders: 7→6→5→4→3→2→1→0
    expect(steps).toEqual([7, 6, 5, 4, 3, 2, 1, 0]);
  });

  it("skips confirmation (step 3) going backwards when no drive folders", () => {
    useAppStore.setState({ onboardingStep: 7 });
    const steps: number[] = [];
    for (let i = 0; i < 8; i++) {
      steps.push(useAppStore.getState().onboardingStep);
      useAppStore.getState().prevStep();
    }
    // Without drive folders: 7→6→5→4→2→1→0→0
    expect(steps).toEqual([7, 6, 5, 4, 2, 1, 0, 0]);
  });

  it("stops at step 0", () => {
    useAppStore.setState({ onboardingStep: 1 });
    useAppStore.getState().prevStep();
    expect(useAppStore.getState().onboardingStep).toBe(0);
    useAppStore.getState().prevStep();
    expect(useAppStore.getState().onboardingStep).toBe(0);
  });
});

// ── Round-trip: adding folders mid-flow ────────────────────────────────────

describe("round-trip with folder changes", () => {
  it("sees confirm page when folders added at step 2 then going forward", () => {
    // Arrive at step 2 with no folders, add one
    useAppStore.setState({ onboardingStep: 2, pendingDriveFolders: [] });
    useAppStore.getState().addDriveFolder(driveFolder("desktop"));
    useAppStore.getState().nextStep();
    expect(useAppStore.getState().onboardingStep).toBe(3);
  });

  it("skips confirm page when all folders removed at step 2", () => {
    useAppStore.setState({
      onboardingStep: 2,
      pendingDriveFolders: [driveFolder("desktop")],
    });
    useAppStore.getState().removeDriveFolder("desktop");
    useAppStore.getState().nextStep();
    expect(useAppStore.getState().onboardingStep).toBe(4);
  });
});

// ── Confirmation page timing ───────────────────────────────────────────────

describe("confirmation page timing", () => {
  it("shows confirmation page only when drive folders exist at step 2", () => {
    // Without folders: step 2 → nextStep → step 4 (skip 3)
    useAppStore.setState({ onboardingStep: 2, pendingDriveFolders: [] });
    useAppStore.getState().nextStep();
    expect(useAppStore.getState().onboardingStep).toBe(4);

    // Reset and add folders: step 2 → nextStep → step 3 (show confirm)
    useAppStore.setState({ onboardingStep: 2 });
    useAppStore.getState().toggleDriveFolder("desktop");
    useAppStore.getState().nextStep();
    expect(useAppStore.getState().onboardingStep).toBe(3);
  });

  it("shows confirmation page when multiple drive folders selected", () => {
    useAppStore.setState({ onboardingStep: 2 });
    useAppStore.getState().toggleDriveFolder("desktop");
    useAppStore.getState().toggleDriveFolder("documents");
    useAppStore.getState().toggleDriveFolder("downloads");
    useAppStore.getState().nextStep();
    expect(useAppStore.getState().onboardingStep).toBe(3);
  });

  it("skipToStep can jump directly to confirmation", () => {
    useAppStore.getState().skipToStep(3);
    expect(useAppStore.getState().onboardingStep).toBe(3);
  });

  it("skipToStep can jump over confirmation", () => {
    useAppStore.getState().skipToStep(5);
    useAppStore.getState().prevStep();
    // Going back from 5 → 4 (no skip, because step ≠ 4)
    expect(useAppStore.getState().onboardingStep).toBe(4);
  });
});

// ── Edge cases ─────────────────────────────────────────────────────────────

describe("edge cases", () => {
  it("photos folders do not affect confirmation page skip", () => {
    useAppStore.setState({ onboardingStep: 2, pendingDriveFolders: [] });
    useAppStore.getState().togglePhotosFolder("pictures");
    // Next step: should still skip confirm because no DRIVE folders
    useAppStore.getState().nextStep();
    expect(useAppStore.getState().onboardingStep).toBe(4);
  });

  it("completeOnboarding transitions to main", () => {
    useAppStore.getState().completeOnboarding();
    expect(useAppStore.getState().phase).toBe("main");
  });
});
