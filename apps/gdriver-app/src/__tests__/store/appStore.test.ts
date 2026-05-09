import { describe, it, expect, beforeEach } from "vitest";
import { useAppStore } from "@/store/appStore";
import type { FolderItem } from "@/store/appStore";

function makeFolder(overrides: Partial<FolderItem> = {}): FolderItem {
  return {
    id: "test-folder",
    name: "Test Folder",
    path: "~/Test",
    type: "drive",
    ...overrides,
  };
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

// ── Initial state ──────────────────────────────────────────────────────────

describe("initial state", () => {
  it("starts in onboarding phase", () => {
    expect(useAppStore.getState().phase).toBe("onboarding");
  });

  it("starts at step 0", () => {
    expect(useAppStore.getState().onboardingStep).toBe(0);
  });

  it("has empty folder lists", () => {
    const s = useAppStore.getState();
    expect(s.pendingDriveFolders).toEqual([]);
    expect(s.pendingPhotosFolders).toEqual([]);
  });

  it("defaults to home page", () => {
    expect(useAppStore.getState().currentPage).toBe("home");
  });

  it("has no open dialog", () => {
    expect(useAppStore.getState().openDialog).toBeNull();
  });
});

// ── Phase & page ───────────────────────────────────────────────────────────

describe("setPhase", () => {
  it("switches to main phase", () => {
    useAppStore.getState().setPhase("main");
    expect(useAppStore.getState().phase).toBe("main");
  });

  it("switches back to onboarding", () => {
    useAppStore.getState().setPhase("main");
    useAppStore.getState().setPhase("onboarding");
    expect(useAppStore.getState().phase).toBe("onboarding");
  });
});

describe("setCurrentPage", () => {
  it.each(["home", "sync", "notifications"] as const)(
    "sets page to %s",
    (page) => {
      useAppStore.getState().setCurrentPage(page);
      expect(useAppStore.getState().currentPage).toBe(page);
    },
  );
});

describe("setOpenDialog", () => {
  it("opens a dialog", () => {
    useAppStore.getState().setOpenDialog("about");
    expect(useAppStore.getState().openDialog).toBe("about");
  });

  it("closes a dialog with null", () => {
    useAppStore.getState().setOpenDialog("preferences");
    useAppStore.getState().setOpenDialog(null);
    expect(useAppStore.getState().openDialog).toBeNull();
  });

  it("sets all dialog types", () => {
    const dialogs = [
      "preferences",
      "offline-files",
      "error-list",
      "about",
      "feedback",
      "account-prefs",
    ] as const;
    for (const d of dialogs) {
      useAppStore.getState().setOpenDialog(d);
      expect(useAppStore.getState().openDialog).toBe(d);
    }
  });
});

// ── Onboarding navigation ──────────────────────────────────────────────────

describe("nextStep", () => {
  it("advances by 1 normally", () => {
    useAppStore.getState().nextStep();
    expect(useAppStore.getState().onboardingStep).toBe(1);
  });

  it("advances multiple times", () => {
    const s = useAppStore.getState();
    s.nextStep();
    s.nextStep();
    expect(useAppStore.getState().onboardingStep).toBe(2);
  });

  it("skips step 3 (confirm) when no drive folders at step 2", () => {
    useAppStore.setState({ onboardingStep: 2, pendingDriveFolders: [] });
    useAppStore.getState().nextStep();
    expect(useAppStore.getState().onboardingStep).toBe(4);
  });

  it("does NOT skip step 3 when drive folders are selected at step 2", () => {
    useAppStore.setState({
      onboardingStep: 2,
      pendingDriveFolders: [makeFolder()],
    });
    useAppStore.getState().nextStep();
    expect(useAppStore.getState().onboardingStep).toBe(3);
  });
});

describe("prevStep", () => {
  it("goes back by 1 normally", () => {
    useAppStore.setState({ onboardingStep: 3 });
    useAppStore.getState().prevStep();
    expect(useAppStore.getState().onboardingStep).toBe(2);
  });

  it("never goes below 0", () => {
    useAppStore.getState().prevStep();
    expect(useAppStore.getState().onboardingStep).toBe(0);
  });

  it("skips step 3 (confirm) when going back from step 4 with no drive folders", () => {
    useAppStore.setState({
      onboardingStep: 4,
      pendingDriveFolders: [],
    });
    useAppStore.getState().prevStep();
    expect(useAppStore.getState().onboardingStep).toBe(2);
  });

  it("does NOT skip step 3 when going back from step 4 with drive folders", () => {
    useAppStore.setState({
      onboardingStep: 4,
      pendingDriveFolders: [makeFolder()],
    });
    useAppStore.getState().prevStep();
    expect(useAppStore.getState().onboardingStep).toBe(3);
  });

  it("goes back normally from step 2", () => {
    useAppStore.setState({ onboardingStep: 2 });
    useAppStore.getState().prevStep();
    expect(useAppStore.getState().onboardingStep).toBe(1);
  });
});

describe("skipToStep", () => {
  it("jumps to target step", () => {
    useAppStore.getState().skipToStep(5);
    expect(useAppStore.getState().onboardingStep).toBe(5);
  });

  it("jumps to step 0", () => {
    useAppStore.setState({ onboardingStep: 10 });
    useAppStore.getState().skipToStep(0);
    expect(useAppStore.getState().onboardingStep).toBe(0);
  });
});

describe("completeOnboarding", () => {
  it("sets phase to main", () => {
    useAppStore.getState().completeOnboarding();
    expect(useAppStore.getState().phase).toBe("main");
  });
});

// ── Drive folders ──────────────────────────────────────────────────────────

describe("drive folder management", () => {
  it("adds a drive folder", () => {
    const f = makeFolder();
    useAppStore.getState().addDriveFolder(f);
    expect(useAppStore.getState().pendingDriveFolders).toEqual([f]);
  });

  it("adds multiple drive folders", () => {
    const a = makeFolder({ id: "a", name: "A" });
    const b = makeFolder({ id: "b", name: "B" });
    useAppStore.getState().addDriveFolder(a);
    useAppStore.getState().addDriveFolder(b);
    expect(useAppStore.getState().pendingDriveFolders).toHaveLength(2);
  });

  it("removes a drive folder by id", () => {
    const f = makeFolder();
    useAppStore.getState().addDriveFolder(f);
    useAppStore.getState().removeDriveFolder(f.id);
    expect(useAppStore.getState().pendingDriveFolders).toEqual([]);
  });

  it("removing unknown id does nothing", () => {
    const f = makeFolder();
    useAppStore.getState().addDriveFolder(f);
    useAppStore.getState().removeDriveFolder("nonexistent");
    expect(useAppStore.getState().pendingDriveFolders).toHaveLength(1);
  });

  it("replaces drive folders", () => {
    useAppStore.getState().addDriveFolder(makeFolder({ id: "old" }));
    useAppStore
      .getState()
      .setDriveFolders([makeFolder({ id: "new" })]);
    expect(useAppStore.getState().pendingDriveFolders).toHaveLength(1);
    expect(useAppStore.getState().pendingDriveFolders[0]!.id).toBe("new");
  });

  it("toggles a suggested folder on (adds it)", () => {
    useAppStore.getState().toggleDriveFolder("desktop");
    const folders = useAppStore.getState().pendingDriveFolders;
    expect(folders).toHaveLength(1);
    expect(folders[0]!.id).toBe("desktop");
    expect(folders[0]!.name).toBe("Desktop");
  });

  it("toggles a suggested folder off (removes it)", () => {
    useAppStore.getState().toggleDriveFolder("desktop");
    useAppStore.getState().toggleDriveFolder("desktop");
    expect(useAppStore.getState().pendingDriveFolders).toEqual([]);
  });

  it("toggling an unknown id has no effect", () => {
    useAppStore.getState().toggleDriveFolder("nonexistent");
    expect(useAppStore.getState().pendingDriveFolders).toEqual([]);
  });

  it("toggles all three suggested drive folders", () => {
    for (const id of ["desktop", "documents", "downloads"]) {
      useAppStore.getState().toggleDriveFolder(id);
    }
    expect(useAppStore.getState().pendingDriveFolders).toHaveLength(3);
  });
});

// ── Photos folders ─────────────────────────────────────────────────────────

describe("photos folder management", () => {
  it("adds a photos folder", () => {
    const f = makeFolder({ id: "pic", type: "photos" });
    useAppStore.getState().addPhotosFolder(f);
    expect(useAppStore.getState().pendingPhotosFolders).toEqual([f]);
  });

  it("removes a photos folder by id", () => {
    const f = makeFolder({ id: "pic", type: "photos" });
    useAppStore.getState().addPhotosFolder(f);
    useAppStore.getState().removePhotosFolder("pic");
    expect(useAppStore.getState().pendingPhotosFolders).toEqual([]);
  });

  it("replaces photos folders", () => {
    useAppStore.getState().addPhotosFolder(makeFolder({ id: "old", type: "photos" }));
    useAppStore
      .getState()
      .setPhotosFolders([makeFolder({ id: "new", type: "photos" })]);
    expect(useAppStore.getState().pendingPhotosFolders).toHaveLength(1);
    expect(useAppStore.getState().pendingPhotosFolders[0]!.id).toBe("new");
  });

  it("toggles a suggested photos folder on", () => {
    useAppStore.getState().togglePhotosFolder("pictures");
    const folders = useAppStore.getState().pendingPhotosFolders;
    expect(folders).toHaveLength(1);
    expect(folders[0]!.id).toBe("pictures");
  });

  it("toggles a suggested photos folder off", () => {
    useAppStore.getState().togglePhotosFolder("pictures");
    useAppStore.getState().togglePhotosFolder("pictures");
    expect(useAppStore.getState().pendingPhotosFolders).toEqual([]);
  });

  it("toggling an unknown photos id has no effect", () => {
    useAppStore.getState().togglePhotosFolder("nonexistent");
    expect(useAppStore.getState().pendingPhotosFolders).toEqual([]);
  });
});
