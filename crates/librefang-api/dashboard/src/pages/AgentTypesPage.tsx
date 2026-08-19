import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Boxes, Plus, Pencil, Trash2, Zap, Loader2 } from "lucide-react";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { Modal } from "../components/ui/Modal";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { MarkdownContent } from "../components/ui/MarkdownContent";
import { toastErr } from "../lib/errors";
import { MultiSelectCmdk } from "../components/ui/MultiSelectCmdk";
import { useSkills } from "../lib/queries/skills";
import { useChannels } from "../lib/queries/channels";
import { useTools } from "../lib/queries/agents";
import type {
  AgentType,
  AgentTypeInput,
  AgentTypeSummary,
  EphemeralResult,
} from "../api";
import { useAgentTypes, useAgentType } from "../lib/queries/agentTypes";
import {
  useCreateAgentType,
  useUpdateAgentType,
  useDeleteAgentType,
  useSpawnEphemeral,
} from "../lib/mutations/agentTypes";

const TEXTAREA_CLASS =
  "w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm font-mono leading-relaxed resize-y disabled:opacity-50 focus:outline-none focus:border-brand";
const INPUT_CLASS =
  "w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm disabled:opacity-50 focus:outline-none focus:border-brand";

interface FormState {
  name: string;
  description: string;
  system_prompt: string;
  provider: string;
  model: string;
  tools: string[];
  skills: string[];
  channels: string[];
  simple_model: string;
  medium_model: string;
  complex_model: string;
  simple_threshold: string;
  complex_threshold: string;
}

const EMPTY_FORM: FormState = {
  name: "",
  description: "",
  system_prompt: "",
  provider: "",
  model: "",
  tools: [],
  skills: [],
  channels: [],
  simple_model: "",
  medium_model: "",
  complex_model: "",
  simple_threshold: "",
  complex_threshold: "",
};

function toForm(type: AgentType): FormState {
  return {
    name: type.name ?? "",
    description: type.description ?? "",
    system_prompt: type.system_prompt ?? "",
    provider: type.provider ?? "",
    model: type.model ?? "",
    tools: type.tools ?? [],
    skills: type.skills ?? [],
    channels: type.channels ?? [],
    simple_model: type.routing?.simple_model ?? "",
    medium_model: type.routing?.medium_model ?? "",
    complex_model: type.routing?.complex_model ?? "",
    simple_threshold: type.routing?.simple_threshold != null ? String(type.routing.simple_threshold) : "",
    complex_threshold: type.routing?.complex_threshold != null ? String(type.routing.complex_threshold) : "",
  };
}

function toInput(form: FormState): AgentTypeInput {
  const hasRouting =
    form.simple_model.trim() !== "" ||
    form.medium_model.trim() !== "" ||
    form.complex_model.trim() !== "";
  return {
    name: form.name.trim(),
    description: form.description.trim() || undefined,
    system_prompt: form.system_prompt.trim() || undefined,
    provider: form.provider.trim() || undefined,
    model: form.model.trim() || undefined,
    tools: form.tools,
    skills: form.skills,
    channels: form.channels,
    routing: hasRouting
      ? {
          simple_model: form.simple_model.trim() || "default",
          medium_model: form.medium_model.trim() || "default",
          complex_model: form.complex_model.trim() || "default",
          simple_threshold: Number(form.simple_threshold) || 0,
          complex_threshold: Number(form.complex_threshold) || 0,
        }
      : undefined,
  };
}

export function AgentTypesPage() {
  const { t } = useTranslation();
  const { data: types, isLoading, isFetching, refetch } = useAgentTypes();
  const skillsQuery = useSkills();
  const toolsQuery = useTools();
  const channelsQuery = useChannels();
  const channelOptions = useMemo(
    () => (channelsQuery.data ?? []).map((c: { name: string }) => c.name),
    [channelsQuery.data],
  );
  const skillOptions = useMemo(
    () => (skillsQuery.data ?? []).map((s: { name: string }) => s.name),
    [skillsQuery.data],
  );
  const toolOptions = useMemo(
    () => (toolsQuery.data ?? []).map((td: { name: string }) => td.name),
    [toolsQuery.data],
  );

  const createType = useCreateAgentType();
  const updateType = useUpdateAgentType();
  const deleteType = useDeleteAgentType();
  const spawn = useSpawnEphemeral();

  // Create/edit dialog. `editing` is null while creating, or the type name
  // while editing (the name field is locked on edit so the PUT path stays
  // stable). The edit form is populated from the detail fetch below.
  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);

  // Detail fetch that backs the edit form. Disabled until a name is selected.
  const detail = useAgentType(editing ?? "");
  // Track which type we've loaded so a re-render can't clobber in-progress edits.
  const loadedFor = useRef<string | null>(null);
  useEffect(() => {
    if (editing && detail.data && loadedFor.current !== editing) {
      setForm(toForm(detail.data));
      loadedFor.current = editing;
    }
  }, [editing, detail.data]);

  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);

  // Quick-run dialog.
  const [runTarget, setRunTarget] = useState<string | null>(null);
  const [runMessage, setRunMessage] = useState("");
  const [runResult, setRunResult] = useState<EphemeralResult | null>(null);

  function openCreate() {
    setEditing(null);
    loadedFor.current = null;
    setForm(EMPTY_FORM);
    setFormOpen(true);
  }

  function openEdit(type: AgentTypeSummary) {
    setEditing(type.name);
    loadedFor.current = null;
    setForm(EMPTY_FORM);
    setFormOpen(true);
  }

  function submitForm() {
    const input = toInput(form);
    if (!input.name) return;
    if (editing) {
      updateType.mutate(
        { name: editing, body: input },
        {
          onSuccess: () => setFormOpen(false),
          onError: (e) => toastErr(e, t("agentTypes.edit")),
        },
      );
    } else {
      createType.mutate(input, {
        onSuccess: () => setFormOpen(false),
        onError: (e) => toastErr(e, t("agentTypes.create")),
      });
    }
  }

  function openRun(type: AgentTypeSummary) {
    setRunTarget(type.name);
    setRunMessage("");
    setRunResult(null);
  }

  function submitRun() {
    if (!runTarget || !runMessage.trim()) return;
    spawn.mutate(
      { agent_type: runTarget, message: runMessage.trim() },
      {
        onSuccess: (res) => setRunResult(res),
        onError: (e) => toastErr(e, t("agentTypes.quickRun")),
      },
    );
  }

  const formPending = createType.isPending || updateType.isPending;
  const editLoading = !!editing && detail.isLoading;

  return (
    <div className="space-y-6">
      <PageHeader
        icon={<Boxes className="h-5 w-5" />}
        title={t("agentTypes.title")}
        isFetching={isFetching}
        onRefresh={() => refetch()}
        actions={
          <Button variant="primary" size="sm" onClick={openCreate}>
            <Plus className="h-4 w-4" />
            {t("agentTypes.create")}
          </Button>
        }
      />

      {isLoading ? (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
          <CardSkeleton />
          <CardSkeleton />
          <CardSkeleton />
        </div>
      ) : !types || types.length === 0 ? (
        <EmptyState
          icon={<Boxes className="h-6 w-6" />}
          title={t("agentTypes.noTypes")}
          action={
            <Button variant="primary" size="sm" onClick={openCreate}>
              <Plus className="h-4 w-4" />
              {t("agentTypes.create")}
            </Button>
          }
        />
      ) : (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {types.map((type) => (
            <Card key={type.name} padding="md" className="flex flex-col gap-3">
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <h3 className="truncate text-sm font-bold text-text-main">
                    {type.name}
                  </h3>
                  {type.description && (
                    <p className="mt-1 line-clamp-2 text-xs text-text-dim">
                      {type.description}
                    </p>
                  )}
                </div>
                {type.source && <Badge variant="brand">{type.source}</Badge>}
              </div>

              <div className="mt-auto flex items-center gap-2 pt-1">
                <Button variant="primary" size="sm" onClick={() => openRun(type)}>
                  <Zap className="h-3.5 w-3.5" />
                  {t("agentTypes.quickRun")}
                </Button>
                {type.source === "agent" ? (
                  <span className="text-[10px] uppercase tracking-wide text-text-dim">
                    {t("agentTypes.managedViaAgents")}
                  </span>
                ) : (
                  <>
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label={t("agentTypes.edit")}
                      onClick={() => openEdit(type)}
                    >
                      <Pencil className="h-3.5 w-3.5" />
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      aria-label={t("agentTypes.delete")}
                      onClick={() => setDeleteTarget(type.name)}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
                  </>
                )}
              </div>
            </Card>
          ))}
        </div>
      )}

      {/* Create / edit dialog — structured form */}
      <Modal
        isOpen={formOpen}
        onClose={() => setFormOpen(false)}
        title={editing ? t("agentTypes.edit") : t("agentTypes.create")}
        size="lg"
      >
        {editLoading ? (
          <div className="flex h-48 items-center justify-center">
            <Loader2 className="h-5 w-5 animate-spin text-text-dim" />
          </div>
        ) : (
          <div className="space-y-4">
            <Input
              label={t("agentTypes.name")}
              value={form.name}
              disabled={!!editing}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
            />
            <Input
              label={t("agentTypes.description")}
              value={form.description}
              onChange={(e) =>
                setForm({ ...form, description: e.target.value })
              }
            />
            <div className="flex flex-col gap-1.5">
              <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                {t("agentTypes.systemPrompt")}
              </label>
              <textarea
                value={form.system_prompt}
                rows={6}
                disabled={formPending}
                onChange={(e) =>
                  setForm({ ...form, system_prompt: e.target.value })
                }
                className={TEXTAREA_CLASS}
              />
            </div>
            <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
              <div className="flex flex-col gap-1.5">
                <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                  {t("agentTypes.provider")}
                </label>
                <input
                  value={form.provider}
                  disabled={formPending}
                  onChange={(e) => setForm({ ...form, provider: e.target.value })}
                  placeholder={t("agentTypes.providerHint")}
                  className={INPUT_CLASS}
                />
              </div>
              <div className="flex flex-col gap-1.5">
                <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                  {t("agentTypes.model")}
                </label>
                <input
                  value={form.model}
                  disabled={formPending}
                  onChange={(e) => setForm({ ...form, model: e.target.value })}
                  placeholder={t("agentTypes.modelHint")}
                  className={INPUT_CLASS}
                />
              </div>
            </div>
            <div className="flex flex-col gap-1.5">
              <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                {t("agentTypes.skills")}
              </label>
              <MultiSelectCmdk
                options={skillOptions}
                value={form.skills}
                onChange={(next) => {
                  const nextValue =
                    typeof next === "function" ? next(form.skills) : next;
                  setForm({ ...form, skills: nextValue });
                }}
                placeholder={t("agentTypes.skillsPlaceholder")}
                allowFreeText
              />
            </div>
            <div className="flex flex-col gap-1.5">
              <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                {t("agentTypes.channels")}
              </label>
              <MultiSelectCmdk
                options={channelOptions}
                value={form.channels}
                onChange={(next) => {
                  const nextValue =
                    typeof next === "function" ? next(form.channels) : next;
                  setForm({ ...form, channels: nextValue });
                }}
                placeholder={t("agentTypes.channelsPlaceholder")}
                allowFreeText
              />
            </div>
            <div className="flex flex-col gap-1.5">
              <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                {t("agentTypes.tools")}
              </label>
              <MultiSelectCmdk
                options={toolOptions}
                value={form.tools}
                onChange={(next) => {
                  const nextValue =
                    typeof next === "function" ? next(form.tools) : next;
                  setForm({ ...form, tools: nextValue });
                }}
                placeholder={t("agentTypes.toolsPlaceholder")}
                allowFreeText
              />
            </div>
            <div className="rounded-lg border border-border-subtle bg-surface/40 p-3 space-y-3">
              <p className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                {t("agentTypes.preferredModels")}
              </p>
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
                <input
                  value={form.simple_model}
                  disabled={formPending}
                  onChange={(e) => setForm({ ...form, simple_model: e.target.value })}
                  placeholder={t("agentTypes.simpleModel")}
                  className={INPUT_CLASS}
                />
                <input
                  value={form.medium_model}
                  disabled={formPending}
                  onChange={(e) => setForm({ ...form, medium_model: e.target.value })}
                  placeholder={t("agentTypes.mediumModel")}
                  className={INPUT_CLASS}
                />
                <input
                  value={form.complex_model}
                  disabled={formPending}
                  onChange={(e) => setForm({ ...form, complex_model: e.target.value })}
                  placeholder={t("agentTypes.complexModel")}
                  className={INPUT_CLASS}
                />
              </div>
              <div className="grid grid-cols-2 gap-3">
                <input
                  type="number"
                  value={form.simple_threshold}
                  disabled={formPending}
                  onChange={(e) => setForm({ ...form, simple_threshold: e.target.value })}
                  placeholder={t("agentTypes.simpleThreshold")}
                  className={INPUT_CLASS}
                />
                <input
                  type="number"
                  value={form.complex_threshold}
                  disabled={formPending}
                  onChange={(e) => setForm({ ...form, complex_threshold: e.target.value })}
                  placeholder={t("agentTypes.complexThreshold")}
                  className={INPUT_CLASS}
                />
              </div>
            </div>
            <div className="flex justify-end gap-2 pt-2">
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setFormOpen(false)}
              >
                {t("common.cancel", { defaultValue: "Cancel" })}
              </Button>
              <Button
                variant="primary"
                size="sm"
                disabled={!form.name.trim() || formPending}
                onClick={submitForm}
              >
                {formPending && <Loader2 className="h-4 w-4 animate-spin" />}
                {t("common.save", { defaultValue: "Save" })}
              </Button>
            </div>
          </div>
        )}
      </Modal>

      {/* Quick-run dialog */}
      <Modal
        isOpen={runTarget !== null}
        onClose={() => setRunTarget(null)}
        title={`${t("agentTypes.quickRun")} — ${runTarget ?? ""}`}
        size="lg"
      >
        <div className="space-y-4">
          <div className="flex flex-col gap-1.5">
            <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
              {t("agentTypes.message")}
            </label>
            <textarea
              value={runMessage}
              rows={4}
              disabled={spawn.isPending}
              onChange={(e) => setRunMessage(e.target.value)}
              className={TEXTAREA_CLASS}
            />
          </div>

          {runResult && (
            <div className="space-y-1.5">
              <label className="text-[10px] font-black uppercase tracking-widest text-text-dim">
                {t("agentTypes.result")}
              </label>
              <div className="max-h-[40vh] overflow-auto rounded-lg border border-border-subtle bg-main px-3 py-2 text-sm">
                <MarkdownContent>{runResult.response}</MarkdownContent>
              </div>
              <div className="flex flex-wrap gap-3 text-[11px] text-text-dim">
                <span>
                  {t("agentTypes.iterations")}: {runResult.iterations}
                </span>
                <span>
                  {t("agentTypes.latency")}: {runResult.latency_ms} ms
                </span>
                {runResult.cost_usd !== null && (
                  <span>
                    {t("agentTypes.cost")}: ${runResult.cost_usd.toFixed(4)}
                  </span>
                )}
              </div>
            </div>
          )}

          <div className="flex justify-end gap-2 pt-2">
            <Button variant="ghost" size="sm" onClick={() => setRunTarget(null)}>
              {t("common.close", { defaultValue: "Close" })}
            </Button>
            <Button
              variant="primary"
              size="sm"
              disabled={!runMessage.trim() || spawn.isPending}
              onClick={submitRun}
            >
              {spawn.isPending ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Zap className="h-4 w-4" />
              )}
              {t("agentTypes.quickRun")}
            </Button>
          </div>
        </div>
      </Modal>

      <ConfirmDialog
        isOpen={deleteTarget !== null}
        title={t("agentTypes.delete")}
        message={t("agentTypes.confirmDelete")}
        confirmLabel={t("agentTypes.delete")}
        tone="destructive"
        onClose={() => setDeleteTarget(null)}
        onConfirm={() => {
          if (!deleteTarget) return;
          deleteType.mutate(deleteTarget, {
            onSuccess: () => setDeleteTarget(null),
            onError: (e) => toastErr(e, t("agentTypes.delete")),
          });
        }}
      />
    </div>
  );
}
