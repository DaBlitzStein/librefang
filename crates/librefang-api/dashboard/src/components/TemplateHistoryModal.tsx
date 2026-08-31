import { useState } from "react";
import { useTranslation } from "react-i18next";
import { History, RotateCcw } from "lucide-react";
import type { TemplateVersion } from "../api";
import { useTemplateHistory } from "../lib/queries/agentTypes";
import { useRestoreTemplateVersion } from "../lib/mutations/agentTypes";
import { Modal } from "./ui/Modal";
import { Button } from "./ui/Button";
import { Badge } from "./ui/Badge";
import { EmptyState } from "./ui/EmptyState";
import { ListSkeleton } from "./ui/Skeleton";
import { ErrorState } from "./ui/ErrorState";
import { ConfirmDialog } from "./ui/ConfirmDialog";
import { useUIStore } from "../lib/store";
import { toastErr } from "../lib/errors";

export function TemplateHistoryModal({
  name,
  open,
  onClose,
}: {
  name: string;
  open: boolean;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const history = useTemplateHistory(name, { enabled: open });
  const restoreMutation = useRestoreTemplateVersion();

  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [pendingRestore, setPendingRestore] = useState<TemplateVersion | null>(null);

  const versions = history.data ?? [];

  async function confirmRestore() {
    if (!pendingRestore) return;
    try {
      await restoreMutation.mutateAsync({ name, versionId: pendingRestore.id });
      addToast(t("agentTypes.saved"), "success");
      setPendingRestore(null);
      onClose();
    } catch (err) {
      addToast(toastErr(err, t("agentTypes.save_failed")), "error");
    }
  }

  return (
    <Modal
      isOpen={open}
      onClose={onClose}
      variant="panel-right"
      size="lg"
      title={t("templateHistory.title", { name })}
    >
      <div className="space-y-4 p-4">
        {history.isLoading ? (
          <ListSkeleton rows={4} />
        ) : history.isError ? (
          <ErrorState message={history.error?.message} onRetry={() => void history.refetch()} />
        ) : versions.length === 0 ? (
          <EmptyState icon={<History className="h-5 w-5" />} title={t("templateHistory.empty")} />
        ) : (
          <div className="space-y-2">
            {versions.map((v) => (
              <div
                key={v.id}
                className="rounded-xl border border-border-subtle bg-surface px-3 py-2.5"
              >
                <div className="flex items-center justify-between gap-2">
                  <div className="flex min-w-0 items-center gap-2">
                    <span className="truncate text-[13px] font-semibold text-text-main">
                      {new Date(v.created_at).toLocaleString()}
                    </span>
                    <Badge variant="default">{`${t("templateHistory.source")}: ${v.source}`}</Badge>
                  </div>
                  <div className="flex shrink-0 items-center gap-1">
                    <button
                      type="button"
                      onClick={() => setExpandedId(expandedId === v.id ? null : v.id)}
                      className="rounded-lg px-2 py-1 text-[11px] text-text-dim hover:bg-main/50 hover:text-text-main"
                    >
                      {expandedId === v.id ? t("common.close") : t("common.details", { defaultValue: "Details" })}
                    </button>
                    <Button
                      variant="secondary"
                      size="sm"
                      leftIcon={<RotateCcw className="h-3.5 w-3.5" />}
                      onClick={() => setPendingRestore(v)}
                    >
                      {t("templateHistory.restore")}
                    </Button>
                  </div>
                </div>
                {expandedId === v.id && (
                  <pre className="mt-2 max-h-64 overflow-auto rounded-lg border border-border-subtle bg-main px-3 py-2 text-[11px] font-mono text-text whitespace-pre-wrap">
                    {v.toml_snapshot}
                  </pre>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      <ConfirmDialog
        isOpen={pendingRestore !== null}
        onClose={() => setPendingRestore(null)}
        onConfirm={() => void confirmRestore()}
        title={t("templateHistory.restore")}
        message={t("templateHistory.restoreConfirm")}
      />
    </Modal>
  );
}
