import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Boxes, Plus, Pencil, Trash2, Zap, Loader2 } from "lucide-react";
import type { TomlTable } from "smol-toml";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { Modal } from "../components/ui/Modal";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { MarkdownContent } from "../components/ui/MarkdownContent";
import { toastErr } from "../lib/errors";
import { MultiSelectCmdk } from "../components/ui/MultiSelectCmdk";
import { AgentManifestForm } from "../components/AgentManifestForm";
import { useSkills } from "../lib/queries/skills";
import { useChannels } from "../lib/queries/channels";
import { useTools } from "../lib/queries/agents";
import { useProviders } from "../lib/queries/providers";
import { useModels } from "../lib/queries/models";
import { useMcpServers } from "../lib/queries/mcp";
import { isProviderAvailable } from "../lib/status";
import {
  emptyManifestExtras,
  emptyManifestForm,
  parseManifestToml,
  serializeManifestForm,
  validateManifestForm,
  type ManifestExtras,
  type ManifestFormState,
} from "../lib/agentManifest";
import type {
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

// `channels` is a plain `AgentManifest` top-level field, but it is NOT part
// of `ManifestFormState` — `AgentManifestForm` has no channels widget
// because the live-agent editor (AgentsPage) manages channels through its
// own dedicated `ChannelsSection`, which talks to the agent-only
// `/agents/{id}/channels` endpoint (there is no running instance for a
// agent type to reach). Agent types have nothing to reach either, so this page
// tracks the allowlist as its own draft and folds it back into
// `extras.topLevel` right before serializing — it round-trips through
// `parseManifestToml`/`serializeManifestForm` exactly like every other
// extras entry the form doesn't own.
const asChannelsArray = (v: unknown): string[] =>
  Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];

const withChannelsExtra = (topLevel: TomlTable, channels: string[]): TomlTable => {
  if (channels.length === 0) {
    const { channels: _drop, ...rest } = topLevel;
    return rest;
  }
  return { ...topLevel, channels };
};

export function AgentTypesPage() {
  const { t } = useTranslation();
  const { data: types, isLoading, isFetching, refetch } = useAgentTypes();
  const skillsQuery = useSkills();
  const toolsQuery = useTools();
  const channelsQuery = useChannels();
  const providersQuery = useProviders();

  const channelOptions = useMemo(
    () => (channelsQuery.data ?? []).map((c: { name: string }) => c.name),
    [channelsQuery.data],
  );
  const skillCatalog = useMemo(
    () =>
      (skillsQuery.data ?? []).map((s: { name: string; description?: string }) => ({
        name: s.name,
        description: s.description,
      })),
    [skillsQuery.data],
  );
  const toolCatalog = useMemo(
    () =>
      (toolsQuery.data ?? []).map((td: { name: string; description?: string }) => ({
        name: td.name,
        description: td.description,
      })),
    [toolsQuery.data],
  );
  const configuredProviders = useMemo(
    () => (providersQuery.data ?? []).filter((p) => isProviderAvailable(p.auth_status)),
    [providersQuery.data],
  );
  const providerOptions = useMemo(
    () => configuredProviders.map((p) => ({ name: p.id })),
    [configuredProviders],
  );

  const createType = useCreateAgentType();
  const updateType = useUpdateAgentType();
  const deleteType = useDeleteAgentType();
  const spawn = useSpawnEphemeral();

  // Create/edit dialog. `editing` is null while creating, or the type name
  // while editing (the name field is locked on edit so the PUT path stays
  // stable). The full-manifest form is seeded from the detail fetch's
  // `manifest_toml` below (#7742 parity with the running-agent editor).
  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  const [formState, setFormState] = useState<ManifestFormState>(emptyManifestForm);
  const [formExtras, setFormExtras] = useState<ManifestExtras>(emptyManifestExtras);
  const [formChannels, setFormChannels] = useState<string[]>([]);
  const [formErrors, setFormErrors] = useState<Set<string>>(new Set());
  const [parseError, setParseError] = useState<string | null>(null);

  // Detail fetch that backs the edit form. Disabled until a name is selected.
  const detail = useAgentType(editing ?? "");
  // Track which type we've loaded so a re-render can't clobber in-progress edits.
  const loadedFor = useRef<string | null>(null);
  useEffect(() => {
    if (!editing || !detail.data || loadedFor.current === editing) return;
    const toml = detail.data.manifest_toml;
    if (!toml) {
      // Nothing to seed from — leave the blank form in place rather than
      // throwing, so an unexpected response shape doesn't crash the drawer.
      loadedFor.current = editing;
      return;
    }
    const parsed = parseManifestToml(toml);
    if (parsed.ok) {
      setFormState(parsed.form);
      setFormExtras(parsed.extras);
      setFormChannels(asChannelsArray(parsed.extras.topLevel.channels));
      setParseError(null);
    } else {
      setParseError(
        parsed.message === "json_schema_unsafe_integer"
          ? t("agents.form.json_schema_unsafe_integer")
          : parsed.message,
      );
    }
    loadedFor.current = editing;
  }, [editing, detail.data, t]);

  // Model catalog for the form's provider/model pickers — filtered by
  // whichever provider is currently selected, gated to while the dialog
  // is open so it isn't polled at page load.
  const modelsQuery = useModels(
    { provider: formState.model.provider },
    { enabled: formOpen && !!formState.model.provider.trim() },
  );
  const modelOptions = useMemo(
    () =>
      (modelsQuery.data?.models ?? []).map((m) => ({ provider: m.provider, id: m.id })),
    [modelsQuery.data?.models],
  );
  const mcpServersQuery = useMcpServers({ enabled: formOpen, refetchInterval: false });
  const mcpCatalog = useMemo(
    () => (mcpServersQuery.data?.configured ?? []).map((s) => ({ name: s.name })),
    [mcpServersQuery.data],
  );

  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);

  // Quick-run dialog.
  const [runTarget, setRunTarget] = useState<string | null>(null);
  const [runMessage, setRunMessage] = useState("");
  const [runResult, setRunResult] = useState<EphemeralResult | null>(null);

  function openCreate() {
    setEditing(null);
    loadedFor.current = null;
    setFormState(emptyManifestForm());
    setFormExtras(emptyManifestExtras());
    setFormChannels([]);
    setFormErrors(new Set());
    setParseError(null);
    setFormOpen(true);
  }

  function openEdit(type: AgentTypeSummary) {
    setEditing(type.name);
    loadedFor.current = null;
    setFormState(emptyManifestForm());
    setFormExtras(emptyManifestExtras());
    setFormChannels([]);
    setFormErrors(new Set());
    setParseError(null);
    setFormOpen(true);
  }

  function submitForm() {
    const errors = validateManifestForm(formState);
    setFormErrors(new Set(errors));
    if (errors.length > 0) return;
    const extrasToSend: ManifestExtras = {
      ...formExtras,
      topLevel: withChannelsExtra(formExtras.topLevel, formChannels),
    };
    const manifest_toml = serializeManifestForm(formState, extrasToSend);
    const input: AgentTypeInput = { name: formState.name.trim(), manifest_toml };
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
            <Card key={`${type.source}:${type.name}`} padding="md" className="flex flex-col gap-3">
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

      {/* Create / edit dialog — full manifest editor (#7742), reusing
          AgentManifestForm the same way the running-agent editor
          (AgentsPage) does. Seeded from the detail GET's `manifest_toml`
          and saved as a full-manifest replacement via
          `PUT /api/agent-types/{name}` (or the create-time equivalent on
          `POST /api/agent-types`) so every manifest field this form covers —
          not just the old 7-field flat shape — round-trips intact. */}
      <Modal
        isOpen={formOpen}
        onClose={() => setFormOpen(false)}
        title={editing ? t("agentTypes.edit") : t("agentTypes.create")}
        size="2xl"
      >
        {editLoading ? (
          <div className="flex h-48 items-center justify-center">
            <Loader2 className="h-5 w-5 animate-spin text-text-dim" />
          </div>
        ) : parseError ? (
          <p className="text-xs text-error">
            {t("agents.form.toml_parse_error", { msg: parseError })}
          </p>
        ) : (
          <div className="space-y-4">
            <div className="max-h-[65vh] overflow-y-auto pr-1 space-y-4">
              <AgentManifestForm
                value={formState}
                onChange={setFormState}
                providers={providerOptions}
                models={modelOptions}
                invalidFields={formErrors}
                extras={formExtras}
                skillCatalog={skillCatalog}
                toolCatalog={toolCatalog}
                mcpCatalog={mcpCatalog}
                nameLocked={!!editing}
              />
              <div className="rounded-xl border border-border-subtle/60 bg-surface/40 p-3 space-y-2.5">
                <p className="text-[10px] font-bold uppercase tracking-widest text-text-dim">
                  {t("agentTypes.channels")}
                </p>
                <MultiSelectCmdk
                  options={channelOptions}
                  value={formChannels}
                  onChange={(next) => {
                    const nextValue =
                      typeof next === "function" ? next(formChannels) : next;
                    setFormChannels(nextValue);
                  }}
                  placeholder={t("agentTypes.channelsPlaceholder")}
                  allowFreeText
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
                disabled={!formState.name.trim() || formPending}
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
