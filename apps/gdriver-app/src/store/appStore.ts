import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";

export interface FolderItem {
  id: string;
  name: string;
  path: string;
  size?: number;
  type: "drive" | "photos";
  isSuggested?: boolean;
}

export type AppPhase = "onboarding" | "main";
export type MainPage = "home" | "sync" | "notifications";
export type DialogType = "preferences" | "offline-files" | "error-list" | "about" | "feedback" | "account-prefs" | null;

interface AppState {
  phase: AppPhase;
  onboardingStep: number;
  pendingDriveFolders: FolderItem[];
  pendingPhotosFolders: FolderItem[];
  currentPage: MainPage;
  openDialog: DialogType;

  nextStep: () => void;
  prevStep: () => void;
  skipToStep: (n: number) => void;
  setPhase: (phase: AppPhase) => void;
  setCurrentPage: (page: MainPage) => void;
  setOpenDialog: (dialog: DialogType) => void;
  addDriveFolder: (folder: FolderItem) => void;
  removeDriveFolder: (id: string) => void;
  toggleDriveFolder: (id: string) => void;
  setDriveFolders: (folders: FolderItem[]) => void;
  addPhotosFolder: (folder: FolderItem) => void;
  removePhotosFolder: (id: string) => void;
  togglePhotosFolder: (id: string) => void;
  setPhotosFolders: (folders: FolderItem[]) => void;
  completeOnboarding: () => void;
  resetOnboarding: () => void;
}

export const SUGGESTED_DRIVE_FOLDERS: FolderItem[] = [
  { id: "desktop", name: "Desktop", path: "~/Desktop", type: "drive", isSuggested: true },
  { id: "documents", name: "Documents", path: "~/Documents", type: "drive", isSuggested: true },
  { id: "downloads", name: "Downloads", path: "~/Downloads", type: "drive", isSuggested: true },
];

export const SUGGESTED_PHOTOS_FOLDERS: FolderItem[] = [
  { id: "pictures", name: "Pictures", path: "~/Pictures", type: "photos", isSuggested: true },
  { id: "movies", name: "Movies", path: "~/Movies", type: "photos", isSuggested: true },
];

export const useAppStore = create<AppState>((set, get) => ({
  phase: "onboarding",
  onboardingStep: 0,
  pendingDriveFolders: [],
  pendingPhotosFolders: [],
  currentPage: "home",
  openDialog: null,

  nextStep: () => {
    const { onboardingStep, pendingDriveFolders } = get();
    let next = onboardingStep + 1;
    // Step 2 → skip confirm (step 3) if no drive folders selected
    if (onboardingStep === 2 && pendingDriveFolders.length === 0) {
      next = 4;
    }
    set({ onboardingStep: next });
  },

  prevStep: () => {
    const { onboardingStep, pendingDriveFolders } = get();
    let prev = Math.max(0, onboardingStep - 1);
    // Step 4 → skip confirm (step 3) if no drive folders selected
    if (onboardingStep === 4 && pendingDriveFolders.length === 0) {
      prev = 2;
    }
    set({ onboardingStep: prev });
  },

  skipToStep: (n: number) => set({ onboardingStep: n }),

  setPhase: (phase: AppPhase) => set({ phase }),
  setCurrentPage: (page: MainPage) => set({ currentPage: page }),
  setOpenDialog: (dialog: DialogType) => set({ openDialog: dialog }),

  addDriveFolder: (folder: FolderItem) =>
    set((s) => ({ pendingDriveFolders: [...s.pendingDriveFolders, folder] })),

  removeDriveFolder: (id: string) =>
    set((s) => ({ pendingDriveFolders: s.pendingDriveFolders.filter((f) => f.id !== id) })),

  toggleDriveFolder: (id: string) => {
    const { pendingDriveFolders } = get();
    const exists = pendingDriveFolders.find((f) => f.id === id);
    if (exists) {
      set({ pendingDriveFolders: pendingDriveFolders.filter((f) => f.id !== id) });
    } else {
      const folder = SUGGESTED_DRIVE_FOLDERS.find((f) => f.id === id);
      if (folder) set({ pendingDriveFolders: [...pendingDriveFolders, { ...folder }] });
    }
  },

  setDriveFolders: (folders: FolderItem[]) => set({ pendingDriveFolders: folders }),

  addPhotosFolder: (folder: FolderItem) =>
    set((s) => ({ pendingPhotosFolders: [...s.pendingPhotosFolders, folder] })),

  removePhotosFolder: (id: string) =>
    set((s) => ({ pendingPhotosFolders: s.pendingPhotosFolders.filter((f) => f.id !== id) })),

  togglePhotosFolder: (id: string) => {
    const { pendingPhotosFolders } = get();
    const exists = pendingPhotosFolders.find((f) => f.id === id);
    if (exists) {
      set({ pendingPhotosFolders: pendingPhotosFolders.filter((f) => f.id !== id) });
    } else {
      const folder = SUGGESTED_PHOTOS_FOLDERS.find((f) => f.id === id);
      if (folder) set({ pendingPhotosFolders: [...pendingPhotosFolders, { ...folder }] });
    }
  },

  setPhotosFolders: (folders: FolderItem[]) => set({ pendingPhotosFolders: folders }),

  completeOnboarding: async () => {
    const { pendingDriveFolders, pendingPhotosFolders } = get();
    const allFolders = [...pendingDriveFolders, ...pendingPhotosFolders];
    console.log("[onboarding] saving", allFolders.length, "folders to daemon...");
    for (const folder of allFolders) {
      try {
        const result = await invoke("add_sync_folder", {
          path: folder.path,
          folderType: folder.type,
        });
        console.log("[onboarding] saved folder:", folder.path, result);
      } catch (e) {
        console.error("[onboarding] failed to save folder:", folder.path, e);
      }
    }
    set({ phase: "main" });
  },

  resetOnboarding: () => set({
    phase: "onboarding",
    onboardingStep: 0,
    pendingDriveFolders: [],
    pendingPhotosFolders: [],
  }),
}));
