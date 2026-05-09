import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { useAppStore } from "@/store/appStore";
import { GOOGLE_PRIVACY, GOOGLE_TERMS } from "@/lib/urls";
import {
  Dialog,
  DialogHeader,
  DialogContent,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";

export default function FeedbackDialog() {
  const { t } = useTranslation();
  const openDialog = useAppStore((s) => s.openDialog);
  const setOpenDialog = useAppStore((s) => s.setOpenDialog);
  const isOpen = openDialog === "feedback";

  const [feedbackText, setFeedbackText] = useState("");
  const [sendSystemInfo, setSendSystemInfo] = useState(true);
  const [inDiscussionGroup, setInDiscussionGroup] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);

  const handleClose = () => {
    setOpenDialog(null);
    setFeedbackText("");
  };

  const handleSubmit = async () => {
    if (!feedbackText.trim()) return;

    setIsSubmitting(true);
    try {
      await invoke("submit_feedback", {
        text: feedbackText,
        includeLogs: sendSystemInfo,
        allowEmail: inDiscussionGroup,
      });
      handleClose();
    } catch {
      // 忽略错误
    } finally {
      setIsSubmitting(false);
    }
  };

  const handlePrivacyPolicy = () => {
    try {
      invoke("open_url", { url: GOOGLE_PRIVACY });
    } catch {
      // 忽略错误
    }
  };

  const handleTermsOfService = () => {
    try {
      invoke("open_url", { url: GOOGLE_TERMS });
    } catch {
      // 忽略错误
    }
  };

  const isSubmitDisabled = !feedbackText.trim() || isSubmitting;

  return (
    <Dialog open={isOpen} onClose={handleClose} className="w-[560px]">
      <DialogHeader onClose={handleClose}>
        <span className="text-[16px]">{t("dialogs.feedback.title")}</span>
      </DialogHeader>

      <DialogContent className="p-6">
        {/* Description */}
        <p className="mb-4 text-[13px] text-app-text-secondary">
          {t("dialogs.feedback.description")}
        </p>

        {/* Feedback textarea */}
        <textarea
          value={feedbackText}
          onChange={(e) => setFeedbackText(e.target.value)}
          placeholder={t("dialogs.feedback.placeholder")}
          className="mb-4 h-32 w-full rounded-lg border border-app-border bg-app-surface p-3 text-[14px] text-app-text-primary placeholder-app-text-secondary outline-none transition-colors focus:border-app-accent focus:ring-1 focus:ring-app-accent"
        />

        {/* Checkboxes */}
        <div className="mb-4 space-y-3">
          <label className="flex items-start gap-3 cursor-pointer">
            <input
              type="checkbox"
              checked={sendSystemInfo}
              onChange={(e) => setSendSystemInfo(e.target.checked)}
              className="mt-0.5 h-4 w-4 rounded accent-app-accent"
            />
            <span className="text-[13px] text-app-text-primary">
              {t("dialogs.feedback.send_system_info")}
            </span>
          </label>
          <label className="flex items-start gap-3 cursor-pointer">
            <input
              type="checkbox"
              checked={inDiscussionGroup}
              onChange={(e) => setInDiscussionGroup(e.target.checked)}
              className="mt-0.5 h-4 w-4 rounded accent-app-accent"
            />
            <span className="text-[13px] text-app-text-primary">
              {t("dialogs.feedback.in_discussion_group")}
            </span>
          </label>
        </div>

        {/* Privacy notice */}
        <p className="text-[12px] text-app-text-secondary">
          {t("dialogs.feedback.privacy_notice")}{" "}
          <button
            onClick={handlePrivacyPolicy}
            className="text-app-accent hover:underline"
          >
            {t("dialogs.feedback.privacy_policy")}
          </button>{" "}
          {t("dialogs.feedback.and")}{" "}
          <button
            onClick={handleTermsOfService}
            className="text-app-accent hover:underline"
          >
            {t("dialogs.feedback.terms_of_service")}
          </button>
          {t("dialogs.feedback.privacy_suffix")}
        </p>
      </DialogContent>

      <DialogFooter>
        <Button variant="outline" onClick={handleClose}>
          {t("common.cancel")}
        </Button>
        <Button onClick={handleSubmit} disabled={isSubmitDisabled}>
          {isSubmitting ? t("common.submitting") : t("common.submit")}
        </Button>
      </DialogFooter>
    </Dialog>
  );
}
