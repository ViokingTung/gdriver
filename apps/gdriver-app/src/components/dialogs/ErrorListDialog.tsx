import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useAppStore } from "@/store/appStore";
import { GOOGLE_DRIVE_SYNC_ERRORS_HELP } from "@/lib/urls";
import type { SyncError } from "@/types/sync";
import {
  Dialog,
  DialogHeader,
  DialogContent,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { AlertCircle, HelpCircle, RefreshCw, SkipForward, CheckCircle, Loader2 } from "lucide-react";

export default function ErrorListDialog() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const openDialog = useAppStore((s) => s.openDialog);
  const setOpenDialog = useAppStore((s) => s.setOpenDialog);
  const isOpen = openDialog === "error-list";

  const { data: errors = [], isLoading } = useQuery<SyncError[]>({
    queryKey: ["sync-errors"],
    queryFn: () => invoke<SyncError[]>("get_sync_errors"),
    refetchInterval: 15_000,
    enabled: isOpen,
  });

  const [retryingId, setRetryingId] = useState<number | null>(null);
  const [skippingId, setSkippingId] = useState<number | null>(null);

  const handleClose = () => setOpenDialog(null);

  const handleRetry = async (errorId: number) => {
    setRetryingId(errorId);
    try {
      await invoke("retry_sync_error", { errorId });
      queryClient.invalidateQueries({ queryKey: ["sync-errors"] });
    } catch {
      // Error already surfaced in the UI
    } finally {
      setRetryingId(null);
    }
  };

  const handleSkip = async (errorId: number) => {
    setSkippingId(errorId);
    try {
      await invoke("retry_sync_error", { errorId });
      queryClient.invalidateQueries({ queryKey: ["sync-errors"] });
    } catch {
      // Error already surfaced in the UI
    } finally {
      setSkippingId(null);
    }
  };

  const handleLearnMore = () => {
    try {
      invoke("open_url", { url: GOOGLE_DRIVE_SYNC_ERRORS_HELP });
    } catch {
      // Ignore
    }
  };

  const errorMessage = (code: string) => {
    const key = `errors.${code}`;
    const translated = t(key);
    return translated === key ? t("errors.UNKNOWN") : translated;
  };

  const hasErrors = errors.length > 0;

  return (
    <Dialog open={isOpen} onClose={handleClose} className="w-[640px]">
      <DialogHeader onClose={handleClose}>
        <span className="text-[16px]">{t("dialogs.error_list.title")}</span>
      </DialogHeader>

      <DialogContent className="p-0">
        {isLoading ? (
          <div className="flex items-center justify-center py-12">
            <Loader2 className="h-5 w-5 animate-spin text-app-accent" />
          </div>
        ) : hasErrors ? (
          <div className="divide-y divide-app-border">
            {errors.map((error) => (
              <div key={error.id} className="flex items-start gap-4 p-4">
                {/* Error icon */}
                <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-status-danger-bg dark:bg-[var(--color-status-error-bg-dark)]">
                  <AlertCircle className="h-5 w-5 text-status-danger dark:text-[var(--color-status-error-dark)]" />
                </div>

                {/* Error content */}
                <div className="min-w-0 flex-1">
                  <div className="mb-1 flex items-center gap-2">
                    <span className="font-medium text-app-text-primary">
                      {error.file_name ?? error.error_code}
                    </span>
                  </div>
                  <p className="mb-2 text-[13px] text-app-text-secondary">
                    {errorMessage(error.error_code)}
                  </p>
                  <button
                    onClick={handleLearnMore}
                    className="flex items-center gap-1 text-[12px] text-app-accent hover:underline"
                  >
                    <HelpCircle className="h-3.5 w-3.5" />
                    {t("dialogs.error_list.learn_more")}
                  </button>
                </div>

                {/* Action buttons */}
                <div className="flex shrink-0 gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => handleRetry(error.id)}
                    disabled={retryingId === error.id}
                    className="gap-1"
                  >
                    <RefreshCw
                      className={`h-3.5 w-3.5 ${
                        retryingId === error.id ? "animate-spin" : ""
                      }`}
                    />
                    {retryingId === error.id ? t("common.retrying") : t("common.retry")}
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => handleSkip(error.id)}
                    disabled={skippingId === error.id}
                    className="gap-1"
                  >
                    <SkipForward className="h-3.5 w-3.5" />
                    {skippingId === error.id ? t("common.loading") : t("common.skip")}
                  </Button>
                </div>
              </div>
            ))}
          </div>
        ) : (
          <div className="flex flex-col items-center justify-center py-12">
            <div className="mb-4 flex h-16 w-16 items-center justify-center rounded-full bg-status-good-bg dark:bg-[var(--color-status-up-to-date-bg-dark)]">
              <CheckCircle className="h-8 w-8 text-status-good dark:text-[var(--color-status-up-to-date-dark)]" />
            </div>
            <h3 className="mb-2 text-[16px] font-medium text-app-text-primary">
              {t("dialogs.error_list.looks_good")}
            </h3>
            <p className="text-center text-[13px] text-app-text-secondary">
              {t("dialogs.error_list.no_errors")}
            </p>
          </div>
        )}
      </DialogContent>

      <DialogFooter>
        <Button variant="outline" onClick={handleClose}>
          {t("common.done")}
        </Button>
      </DialogFooter>
    </Dialog>
  );
}
