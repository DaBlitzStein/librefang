import { formatRelativeTime, formatSqliteDateTime } from "../lib/datetime";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate, useSearch } from "@tanstack/react-router";
import {
  ALLOWED_AVATAR_TYPES,
  MAX_AVATAR_BYTES,
  type AgentDetail,
  type AgentIdentity,
  type AgentItem,
  type CloneAgentResult,
  type ToolDefinition,
} from "../api";
import { useQueryClient } from "@tanstack/react-query";
import { isProviderAvailable } from "../lib/status";
import { PageHeader } from "../components/ui/PageHeader";
import { CardSkeleton } from "../components/ui/Skeleton";
import { EmptyState } from "../components/ui/EmptyState";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { DrawerPanel } from "../components/ui/DrawerPanel";
import { Modal } from "../components/ui/Modal";
import { useCreateShortcut } from "../lib/useCreateShortcut";
import { MultiSelectCmdk } from "../components/ui/MultiSelectCmdk";
import { Card } from "../components/ui/Card";
import { Input } from "../components/ui/Input";
import { Button } from "../components/ui/Button";
import { Badge, dotColors } from "../components/ui/Badge";
import { Avatar } from "../components/ui/Avatar";
import { AgentAvatar } from "../components/AgentAvatar";
import { PromptsExperimentsPanel } from "../components/PromptsExperimentsPanel";
import { AgentTabBar } from "../components/AgentTabs";
import type { AgentTabDef } from "../components/AgentTabs";
import { AgentBrief } from "../components/AgentBrief";
import type { AgentBriefMessage } from "../components/AgentBrief";
import { QuickRunModal } from "../components/QuickRunModal";
import { useUIStore } from "../lib/store";
import { copyToClipboard } from "../lib/clipboard";
import { toastErr } from "../lib/errors";
import { Search, Users, MessageCircle, X, Wrench, Shield, Plus, Loader2, Pause, Play, Clock, Brain, Zap, FlaskConical, Trash2, Copy, RotateCcw, Pencil, Bot, Database, FileText, MoreHorizontal, Sparkles, Save, GitBranch, History, Radio, Route, Settings, KeyRound } from "lucide-react";
import { truncateId } from "../lib/string";
import { pickLatestSessionId } from "../lib/sessionSelector";
import { getStatusVariant } from "../lib/status";
import { useDashboardSnapshot } from "../lib/queries/overview";
import { useSessionDetails } from "../lib/queries/sessions";
import { useAgentKvMemory } from "../lib/queries/memory";
// Schedule tab's cron + trigger queries live inside <AgentSchedulePanel/>
// (issue #4924). React-Query dedupes identical queryKey subscriptions so
// the panel and the SchedulerPage (when both mounted) share the same cache.
import { useProviders } from "../lib/queries/providers";
import { useModels } from "../lib/queries/models";
import { useSkills } from "../lib/queries/skills";
import { useMcpServers } from "../lib/queries/mcp";
import { useWhoami } from "../lib/queries/authz";
import { AgentManifestForm } from "../components/AgentManifestForm";
import type { ManifestSectionId } from "../components/AgentManifestForm";
import { sectionForInvalidField } from "../components/AgentManifestForm";
import { AgentSchedulePanel } from "../components/AgentSchedulePanel";
import { useModelRouterProfiles } from "../lib/queries/modelRouter";
import {
  emptyManifestExtras,
  emptyManifestForm,
  parseManifestToml,
  preservedWorkspaceNamesFromExtras,
  serializeManifestForm,
  validateManifestForm,
  type ManifestExtras,
  type ManifestFormState,
} from "../lib/agentManifest";
import { generateManifestMarkdown } from "../lib/agentManifestMarkdown";
import {
  agentQueries,
  useAgentEvents,
  useAgentSessions,
  useAgentStats,
  useAgentTemplates,
  useAgentMcpServers,
  useAgentAvatarUrl,
  useAgentManifestHistory,
  useAgentManifest,
  useAgentChannels,
  useTools,
} from "../lib/queries/agents";
import {
  useAgentTemplateToml,
  useCloneAgent,
  useDeleteAgent,
  useDeleteAgentAvatar,
  usePatchAgent,
  useUpdateAgentIdentity,
  useUploadAgentAvatar,
  useResetAgentSession,
  useResumeAgent,
  useSpawnAgent,
  useSuspendAgent,
  useSetAgentChannels,
} from "../lib/mutations/agents";
import { formatNumber } from "../lib/format";

/**
 * Local view type that pairs the strict `AgentDetail` shape from `api.ts`
 * with the additional runtime fields the backend actually returns on
 * `GET /api/agents/{id}` but which haven't been added to the canonical
 * type yet. Keeping this scoped to AgentsPage avoids widening the
 * exported interface for other consumers and removes the need for
 * `(agent as AgentView).field` casts inside the master-detail rendering.
 */
type AgentTriggerSummary = {
  event_pattern?: string;
  name?: string;
  description?: string;
};
type AgentCronSummary = {
  schedule?: string;
  cron?: string;
  expression?: string;
  next_run?: string;
  name?: string;
  id?: string;
};
type AgentView = AgentDetail & {
  state?: string;
  description?: string;
  /** Emoji, colour and avatar reference. `GET /api/agents/{id}` emits these
   *  (`lifecycle.rs: get_agent`) but `AgentDetail` never declared them, which
   *  is the whole reason the drawer rendered initials for everyone (#8339). */
  identity?: AgentIdentity;
  profile?: string;
  model_name?: string;
  model_provider?: string;
  last_active?: string;
  triggers?: AgentTriggerSummary[];
  cron_jobs?: AgentCronSummary[];
  /** Lineage fields the backend only emits on `GET /api/agents` (the list
   *  endpoint's `enrich_agent_json`), not on the per-agent detail fetch.
   *  Carried over from the `AgentItem` row when opening the detail panel. */
  parent_agent_id?: string | null;
  /** Raw `AgentEntry` serde form of the same link, per `AgentItem`'s docs. */
  parent?: string | null;
  parent_unknown?: boolean;
  children?: string[];
  capabilities?: Omit<NonNullable<AgentDetail["capabilities"]>, "tools" | "skills"> & {
    skills?: string[];
    tools?: string[];
  };
};

function safeStringify(v: unknown): string {
  try {
    return JSON.stringify(v);
  } catch {
    return String(v);
  }
}

export function cloneResultNotice(result: CloneAgentResult): {
  partial: boolean;
  warnings: string;
} {
  const warnings = result.warnings.filter(Boolean);
  return {
    partial: result.partial || warnings.length > 0,
    warnings: warnings.join(", ") || "unknown",
  };
}

/**
 * Whether the token-footprint panel has data to show. A genuine zero (a
 * tools-disabled agent with no system_prompt) is real data, not "missing" —
 * only the absence of the field means the daemon has nothing to report.
 *
 * Lives next to the panel that renders it (`AgentBrief`) and is re-exported
 * here so the existing import site and its tests keep working.
 */
export { hasTokenFootprintData } from "../components/AgentBrief";

/** Two-column row used inside the detail modal's value cards. */
function DetailRow({ label, children }: { label: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="flex justify-between items-center gap-3 min-h-[28px]">
      <span className="text-text-dim text-sm">{label}</span>
      <span className="text-sm text-right min-w-0">{children}</span>
    </div>
  );
}

/**
 * The agent's visual identity, editable: emoji, and the avatar image (#8339).
 *
 * There is deliberately no field for `avatar_url`. It may only ever hold this
 * agent's own avatar path — #8349 closed it to that — so the upload and remove
 * buttons are the only two things that write it. A free-text URL box would be a
 * way to make the dashboard fetch from wherever the text said, which is the one
 * thing the backend change exists to prevent.
 *
 * `onChanged` fires after a successful write so the drawer can re-read the
 * agent: the identity lives in the page's local `detailAgent` state, which a
 * query invalidation alone does not reach.
 */
export function AgentAppearanceSection({
  agentId,
  identity,
  onChanged,
}: {
  agentId: string;
  identity?: AgentIdentity;
  onChanged: () => void;
}) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);
  const updateIdentityMutation = useUpdateAgentIdentity();
  const uploadAvatarMutation = useUploadAgentAvatar();
  const deleteAvatarMutation = useDeleteAgentAvatar();
  const storedEmoji = identity?.emoji ?? "";
  const hasAvatar = !!identity?.avatar_url;
  const [emojiDraft, setEmojiDraft] = useState(storedEmoji);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Re-seed on the agent, not on the stored emoji: the drawer stays mounted
  // while the user clicks through the list, and keying this on the value would
  // wipe a half-typed emoji the moment an unrelated poll refreshed the agent.
  useEffect(() => {
    setEmojiDraft(identity?.emoji ?? "");
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the agent, deliberately not on the stored emoji
  }, [agentId]);

  /** PATCH the emoji. Colour is left alone — the body omits it, and #6608 made
   *  an omitted field preserve its stored value rather than null it. */
  function saveEmoji() {
    if (updateIdentityMutation.isPending) return;
    const next = emojiDraft.trim();
    if (next === storedEmoji) return;
    updateIdentityMutation.mutate(
      // The empty string, never `undefined`: omitting a field is how a PATCH
      // says "leave it", so clearing an emoji has to be spelled out.
      { agentId, identity: { emoji: next } },
      {
        onSuccess: () => {
          onChanged();
          addToast(
            next
              ? t("agents.identity.emoji_saved", { defaultValue: "Emoji updated" })
              : t("agents.identity.emoji_cleared", { defaultValue: "Emoji cleared" }),
            "success",
          );
        },
        onError: (e: Error) =>
          addToast(
            e?.message || t("agents.identity.emoji_failed", { defaultValue: "Failed to update the emoji" }),
            "error",
          ),
      },
    );
  }

  /** Validate the picked file, then send its bytes.
   *
   *  Both checks mirror the server's and neither replaces it: the daemon
   *  decides the format by sniffing the bytes, so a `.png` that is really
   *  something else is refused there whatever the browser said here. Checking
   *  first only avoids spending an upload that was never going to be accepted,
   *  and lets the message name the actual problem. */
  function handleFileChange(event: React.ChangeEvent<HTMLInputElement>) {
    const input = event.target;
    const file = input.files?.[0];
    // Cleared before any early return, so picking the same file twice in a row
    // still fires `change` — the value is what the browser compares against.
    input.value = "";
    if (!file) return;

    if (!(ALLOWED_AVATAR_TYPES as readonly string[]).includes(file.type)) {
      addToast(
        t("agents.identity.avatar_type_rejected", {
          defaultValue: "An avatar must be a PNG, JPEG, GIF or WebP image. SVG is not accepted.",
        }),
        "error",
      );
      return;
    }
    if (file.size > MAX_AVATAR_BYTES) {
      addToast(
        t("agents.identity.avatar_too_large", {
          defaultValue: "That image is {{size}} MB; the limit is {{limit}} MB.",
          size: (file.size / (1024 * 1024)).toFixed(1),
          limit: (MAX_AVATAR_BYTES / (1024 * 1024)).toFixed(0),
        }),
        "error",
      );
      return;
    }

    uploadAvatarMutation.mutate(
      { agentId, file },
      {
        onSuccess: () => {
          onChanged();
          addToast(t("agents.identity.avatar_saved", { defaultValue: "Avatar updated" }), "success");
        },
        onError: (e: Error) =>
          addToast(
            e?.message || t("agents.identity.avatar_failed", { defaultValue: "Failed to upload the avatar" }),
            "error",
          ),
      },
    );
  }

  function removeAvatar() {
    if (deleteAvatarMutation.isPending) return;
    deleteAvatarMutation.mutate(agentId, {
      onSuccess: () => {
        onChanged();
        addToast(t("agents.identity.avatar_removed", { defaultValue: "Avatar removed" }), "success");
      },
      onError: (e: Error) =>
        addToast(
          e?.message || t("agents.identity.avatar_remove_failed", { defaultValue: "Failed to remove the avatar" }),
          "error",
        ),
    });
  }

  return (
    <section>
      <h4 className="text-sm font-semibold flex items-center gap-2 mb-2">
        <Sparkles className="w-3.5 h-3.5 text-brand" />
        {t("agents.identity.title", { defaultValue: "Appearance" })}
      </h4>
      <div className="rounded-lg bg-main border border-border-subtle p-4 space-y-2">
        <DetailRow label={t("agents.identity.emoji", { defaultValue: "Emoji" })}>
          <div className="flex items-center gap-2 justify-end">
            <input
              type="text"
              value={emojiDraft}
              onChange={e => setEmojiDraft(e.target.value)}
              onKeyDown={e => {
                // Same `isComposing` guard as the rename field: in a CJK IME
                // Enter confirms the candidate, and submitting on it would
                // hijack the composition.
                if (e.key === "Enter" && !e.nativeEvent.isComposing) saveEmoji();
              }}
              // 16, not 1 or 2: a single emoji is not a single character. A
              // family with ZWJ joiners (👨‍👩‍👧‍👦) is eleven UTF-16 units,
              // and a cap that counted "characters" would truncate it into a
              // different picture.
              maxLength={16}
              placeholder={t("agents.identity.emoji_placeholder", { defaultValue: "None" })}
              aria-label={t("agents.identity.emoji", { defaultValue: "Emoji" })}
              className="w-24 px-2 py-1 rounded-md border border-border-subtle bg-surface text-lg text-center outline-none focus:border-brand"
            />
            <button
              type="button"
              onClick={saveEmoji}
              disabled={updateIdentityMutation.isPending || emojiDraft.trim() === storedEmoji}
              className="px-3 py-1 rounded-lg text-xs font-semibold bg-brand text-white hover:bg-brand/90 disabled:opacity-50 disabled:cursor-not-allowed shrink-0"
            >
              {updateIdentityMutation.isPending ? t("common.saving") : t("common.save")}
            </button>
          </div>
        </DetailRow>
        <DetailRow label={t("agents.identity.avatar", { defaultValue: "Avatar image" })}>
          <div className="flex items-center gap-2 justify-end">
            <input
              type="file"
              ref={fileInputRef}
              accept={ALLOWED_AVATAR_TYPES.join(",")}
              onChange={handleFileChange}
              className="hidden"
              data-testid="agent-avatar-file-input"
              aria-label={t("agents.identity.avatar", { defaultValue: "Avatar image" })}
            />
            <button
              type="button"
              onClick={() => fileInputRef.current?.click()}
              disabled={uploadAvatarMutation.isPending}
              className="px-3 py-1 rounded-lg text-xs font-semibold bg-main hover:bg-main/80 text-text-dim border border-border-subtle disabled:opacity-50 disabled:cursor-not-allowed shrink-0"
            >
              {uploadAvatarMutation.isPending
                ? t("common.saving")
                : hasAvatar
                  ? t("agents.identity.avatar_replace", { defaultValue: "Replace" })
                  : t("agents.identity.avatar_upload", { defaultValue: "Upload" })}
            </button>
            {hasAvatar && (
              <button
                type="button"
                onClick={removeAvatar}
                disabled={deleteAvatarMutation.isPending}
                className="px-3 py-1 rounded-lg text-xs font-semibold text-error border border-error/30 hover:bg-error/10 disabled:opacity-50 disabled:cursor-not-allowed shrink-0"
              >
                {t("common.remove", { defaultValue: "Remove" })}
              </button>
            )}
          </div>
        </DetailRow>
        <p className="text-[11px] text-text-dim leading-relaxed">
          {t("agents.identity.avatar_hint", {
            defaultValue: "PNG, JPEG, GIF or WebP, up to 2 MB. SVG is not accepted. The image is stored by the daemon and served only to signed-in callers.",
          })}
        </p>
      </div>
    </section>
  );
}

/**
 * Whether the signed-in credential may edit an agent's emoji and avatar.
 *
 * The daemon's rule for both writes the appearance section performs is
 * `role >= UserRole::Admin` (middleware.rs), and it reads the *credential's*
 * role — the group-derived ones `whoami` reports separately do not open this
 * door, which is the direction that would hand a viewer controls that can only
 * 403.
 *
 * Pure and exported because `AgentsPage` has no render harness, so a predicate
 * left inline would be covered by nothing.
 */
export function canEditAgentIdentity(role: string | undefined): boolean {
  return role === "admin" || role === "owner";
}

/**
 * What the create drawer should open on when `/agents` was reached with a
 * `template` search param, or `null` when it was not.
 *
 * Pure and exported on purpose: `AgentsPage` has no render harness (it holds
 * some twenty hooks), so a rule left inline in the effect below would be
 * covered by nothing, and the param name is the contract with the sender on
 * `/agent-types`.
 */
export function createDrawerSeed(
  template: string | undefined,
): { createMode: "template"; templateName: string } | null {
  // Falsy covers both "no param" and the empty string `validateSearch` would
  // otherwise admit.
  if (!template) return null;
  return { createMode: "template", templateName: template };
}

/**
 * Channels quick-widget for the Configure drawer (#7742). `PUT
 * /api/agents/{id}/channels` has existed since `config.rs` shipped
 * `get_agent_channels` / `set_agent_channels` (promised when #4912 / #4963
 * closed) but never got a dashboard client — this is that client. Reuses
 * `MultiSelectCmdk`, the same picker skills/tools already use, seeded from
 * the instance's configured channel types.
 */
export function ChannelsSection({ agentId }: { agentId: string }) {
  const { t } = useTranslation();
  const addToast = useUIStore((s) => s.addToast);

  const channelsQuery = useAgentChannels(agentId);
  const setChannels = useSetAgentChannels();

  const assigned = useMemo(() => channelsQuery.data?.assigned ?? [], [channelsQuery.data]);
  const available = channelsQuery.data?.available ?? [];
  const [draft, setDraft] = useState<string[] | null>(null);
  // Reset the draft whenever the agent changes (drawer reused across
  // selections) or the persisted value moves out from under a pristine draft.
  useEffect(() => {
    setDraft(null);
  }, [agentId]);

  const current = draft ?? assigned;
  const dirty =
    draft !== null &&
    (draft.length !== assigned.length || draft.some((c) => !assigned.includes(c)));

  const save = () => {
    if (draft === null || setChannels.isPending) return;
    setChannels.mutate(
      { agentId, channels: draft },
      {
        onSuccess: () => {
          addToast(
            t("agents.detail.channels_saved", { defaultValue: "Channels updated" }),
            "success",
          );
          setDraft(null);
        },
        onError: (e: Error) =>
          addToast(e.message || t("common.error", { defaultValue: "Error" }), "error"),
      },
    );
  };

  return (
    <section>
      <h4 className="text-sm font-semibold mb-2 flex items-center gap-2">
        <Radio className="w-3.5 h-3.5 text-brand" />
        {t("agents.detail.channels", { defaultValue: "Channels" })}
      </h4>
      <p className="text-[11px] text-text-dim mb-2 leading-relaxed">
        {t("agents.detail.channels_hint", {
          defaultValue: "Empty means reachable from every configured channel.",
        })}
      </p>
      {channelsQuery.isLoading ? (
        <p className="text-xs text-text-dim">{t("common.loading", { defaultValue: "Loading..." })}</p>
      ) : /* `available` comes only from `config.sidecar_channels`
             (`routes/agents/config.rs: get_agent_channels`) and never unions in
             the agent's own `channels`, so an allowlist left over from a
             since-removed sidecar channel has a non-empty `assigned` against an
             empty `available`. Gating on `available` alone hid that allowlist
             behind "No channels configured" while it actively restricted the
             agent, with no way to clear it. `MultiSelectCmdk` renders its chips
             from `value`, so an assigned-but-unavailable name still shows and
             still survives a save (#7749 review). */
        available.length > 0 || assigned.length > 0 ? (
        <MultiSelectCmdk
          options={available}
          value={current}
          onChange={(next) => setDraft(typeof next === "function" ? next(current) : next)}
          placeholder={t("agents.detail.channels_search_placeholder", {
            defaultValue: "Search channels…",
          })}
        />
      ) : (
        <p className="text-xs text-text-dim">
          {t("agents.detail.no_channels_configured", {
            defaultValue: "No channels configured on this instance.",
          })}
        </p>
      )}
      {dirty && (
        <div className="flex justify-end gap-2 mt-2">
          <button
            type="button"
            onClick={() => setDraft(null)}
            className="px-3 py-1 rounded-lg text-xs font-semibold bg-main hover:bg-main/80 text-text-dim border border-border-subtle"
          >
            {t("common.cancel")}
          </button>
          <button
            type="button"
            disabled={setChannels.isPending}
            onClick={save}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-brand text-white text-xs font-bold disabled:opacity-40 disabled:cursor-not-allowed hover:bg-brand/90 transition-colors"
          >
            {setChannels.isPending ? (
              <Loader2 className="w-3 h-3 animate-spin" />
            ) : (
              <Save className="w-3 h-3" />
            )}
            {t("common.save")}
          </button>
        </div>
      )}
    </section>
  );
}

/**
 * The two main tabs of the agent detail panel.
 *
 * Every other surface of an agent is reachable from one of these two, and the
 * split is by *what you are doing*, not by which subsystem the field belongs
 * to: `info` reads the agent (what it is doing, what it has been doing, what it
 * remembers), `config` writes it. The eight tabs this replaced were split by
 * subsystem, which is how the token footprint ended up in a drawer and the
 * complexity router two levels below the tab that named it.
 */
export type AgentMainTab = "info" | "config";

/**
 * Sub-tabs of "logs & info", in display order.
 *
 * Read-only surfaces, which is why they live under the reading tab: the
 * manifest editor is the one writer of the agent's configuration, and none of
 * these four carries a field of its own.
 */
export const INFO_TABS = ["logs", "memory", "prompts", "history"] as const;

export type InfoTab = (typeof INFO_TABS)[number];

/**
 * The config groups, in display order.
 *
 * A runtime array rather than a bare union for the same reason
 * `MANIFEST_SECTION_IDS` is one: the guard that checks every group has content
 * and every group has a label can walk the real list instead of restating it.
 */
export const CONFIG_GROUP_IDS = [
  "general",
  "model",
  "permissions",
  "tools",
  "memory",
  "limits",
  "channels",
  "planning",
  "conversation",
] as const;

export type ConfigGroupId = (typeof CONFIG_GROUP_IDS)[number];

/**
 * Which manifest sections each config group hosts.
 *
 * This is the map the whole surface hangs from: the group tabs are drawn from
 * its keys, the form is handed its values, and the guards assert that the two
 * directions agree — every section appears exactly once, and every group
 * appears exactly once in the bar.
 *
 * The grouping is by subject rather than by manifest table, because the table
 * boundaries are an implementation detail of the Rust struct and an operator
 * looking for "where do I cap this agent's spending" does not know that
 * `resources` and `rl_export` are siblings of `limits`.
 *
 * Every section of `AgentManifestForm` is here, including the ones the plan's
 * original table named as separate groups: `tools` and `exec_policy` render
 * inside `capabilities`, `resources` and `rl_export` inside `limits`, and
 * `autonomous` — which that table omitted — is a scheduling concern, so it sits
 * with the cron jobs it governs.
 */
export const CONFIG_GROUPS: Record<ConfigGroupId, readonly ManifestSectionId[]> = {
  general: ["identity", "metadata", "prompt", "lifecycle", "response_format"],
  model: ["model", "fallback_models", "thinking", "routing"],
  // The user's split: model and routing on one tab, and what the agent is
  // *allowed* to do on a tab of its own. `capabilities` grants network hosts,
  // shell commands and tool names; `exec_policy` bounds the shell. Both were
  // elsewhere — capabilities under Tools & skills beside the tools it grants,
  // and exec_policy folded into Lifecycle, where every other switch is a
  // preference rather than a permission.
  permissions: ["capabilities", "exec_policy"],
  tools: ["tools", "skills", "mcp_servers", "skill_workshop"],
  memory: ["proactive_memory", "auto_dream", "compaction", "context_engine"],
  limits: ["limits"],
  channels: ["channel_overrides"],
  planning: ["scheduling", "autonomous", "async_tasks"],
  conversation: ["context_injection", "shared_folders"],
};

/**
 * The config group that owns the first field a validation run complained about.
 *
 * A failed save has to send the operator somewhere they can act. With the
 * sections grouped the offending field is usually in a group they are not
 * looking at, and an error nobody can see is indistinguishable from no error —
 * so this resolves the first reported field path to its section, and that
 * section to the one group that hosts it.
 *
 * Pulled out of `saveManifestEditor` so it can be tested without rendering the
 * page: `AgentsPage` has some twenty hooks and no render harness, so anything
 * left inline there is untestable by construction. The decision is the part
 * with behaviour in it; the caller is one `setConfigGroup`.
 *
 * Returns `undefined` when nothing maps — an unrecognised path (in which case
 * `sectionForInvalidField` has no entry and its guard test fails), or a
 * section no group hosts (in which case the layout guard fails). Every path
 * `validateManifestForm` can produce is covered by one of those two, which is
 * what makes the fallback safe rather than silent.
 */
export function groupForFirstInvalidField(
  errors: readonly string[],
): ConfigGroupId | undefined {
  const firstBadSection = errors
    .map(sectionForInvalidField)
    .find((section): section is ManifestSectionId => section !== undefined);
  if (!firstBadSection) return undefined;

  return CONFIG_GROUP_IDS.find((group) =>
    CONFIG_GROUPS[group].includes(firstBadSection),
  );
}

/**
 * Blocklist wins: drop every allowlist entry the blocklist also names.
 *
 * The kernel applies the blocklist after the allowlist, so the overlapping
 * names were already inert — but leaving them in the file says two different
 * things at once. The form's conflict notice promises those entries "will be
 * removed from the allowlist when you save", and this is that promise: the
 * legacy Tools Editor resolved the overlap the same way in its own Save, and
 * the manifest form inherited the job when that modal was retired. Pure so the
 * decision is testable without the page's twenty hooks.
 */
export function resolveToolListConflicts(
  form: ManifestFormState,
): ManifestFormState {
  if (form.tool_blocklist.length === 0 || form.tool_allowlist.length === 0) {
    return form;
  }
  const blocked = new Set(form.tool_blocklist);
  const resolved = form.tool_allowlist.filter((name) => !blocked.has(name));
  if (resolved.length === form.tool_allowlist.length) return form;
  return { ...form, tool_allowlist: resolved };
}

export function AgentsPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  // The `template` search param, set by the agent-types page's Run button.
  // Deliberately not named `search`: the local state below is the agent filter.
  const { template: routeTemplate } = useSearch({ from: "/agents" });
  const [search, setSearch] = useState("");
  const [detailAgent, setDetailAgent] = useState<AgentDetail | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [createMode, setCreateMode] = useState<"form" | "template" | "toml">("form");
  const [templateName, setTemplateName] = useState("");
  const [templateCustomName, setTemplateCustomName] = useState("");
  const [manifestToml, setManifestToml] = useState("");
  const [templateTomlLoading, setTemplateTomlLoading] = useState(false);
  const [formState, setFormState] = useState<ManifestFormState>(emptyManifestForm);
  const [formExtras, setFormExtras] = useState<ManifestExtras>(emptyManifestExtras);
  const [formErrors, setFormErrors] = useState<Set<string>>(new Set());
  const [tomlParseError, setTomlParseError] = useState<string | null>(null);
  // Parent whose ledger pays for the Quick Run (#6699). Held as an id rather
  // than a flag so the dialog knows which agent to preselect; `null` is closed.
  const [quickRunParent, setQuickRunParent] = useState<string | null>(null);
  // Inline-rename state for the detail/edit modal header. The agent name is
  // the primary identifier in the UI and was previously read-only — now
  // clicking the title swaps it for an input and PATCHes /agents/{id}.
  const [editingName, setEditingName] = useState(false);
  const [nameDraft, setNameDraft] = useState("");
  // Destructive-action confirmation dialog. We set this instead of calling
  // window.confirm() so the dialog matches the rest of the dashboard
  // styling and can slide up as a bottom-sheet on mobile.
  const [confirmDialog, setConfirmDialog] = useState<{
    title: string;
    message: string;
    onConfirm: () => void | Promise<void>;
    tone?: "default" | "destructive";
  } | null>(null);
  // Clone dialog state (#6566). `POST /api/agents/{id}/clone` requires `new_name` with no serde default, so the button has to collect a name before firing — it previously posted `{}` and 422'd on every click.
  const [cloneDialog, setCloneDialog] = useState<{
    agentId: string;
    sourceName: string;
  } | null>(null);
  const [cloneNameDraft, setCloneNameDraft] = useState("");
  const [cloneIncludeSkills, setCloneIncludeSkills] = useState(true);
  const [cloneIncludeTools, setCloneIncludeTools] = useState(true);
  const [showHandAgents, setShowHandAgents] = useState(false);
  const [stateFilter, setStateFilter] = useState<"all" | "running" | "suspended">("all");
  const [sortBy, setSortBy] = useState<"name" | "last_active" | "created_at">("name");
  // The agent view is two tabs deep: the main tab picks reading or writing,
  // the second level picks which sub-surface. Three independent pieces of
  // state rather than one union, because they are remembered independently —
  // leaving config for the logs and coming back should land in the group you
  // were editing, not reset to the first one.
  const [mainTab, setMainTab] = useState<AgentMainTab>("info");
  const [infoTab, setInfoTab] = useState<InfoTab>("logs");
  const [configGroup, setConfigGroup] = useState<ConfigGroupId>("general");
  // The config tab's "advanced mode". One switch for every section's folded
  // half; see `AgentManifestFormProps.advanced` for why it is not per section.
  const [advancedMode, setAdvancedMode] = useState(false);
  // Whether the details drawer is open. Decoupled from `detailAgent` so
  // selecting an agent in the list shows the inline detail panel without
  // popping a drawer; the drawer holds the avatar editor, the agent's
  // read-only lineage and the lifecycle actions, and is opened from the
  // detail header's overflow button.
  const [detailDrawerOpen, setDetailDrawerOpen] = useState(false);
  // The manifest editor (#7742) is the config tab now, so it is live exactly
  // while that tab is selected — see `manifestEditorLive` below.
  // Seeded from `GET /agents/{id}/manifest` (raw TOML) the first time it opens
  // for a given agent; `manifestEditorSeededFor` gates the seed effect so a
  // background refetch of that query (e.g. from an unrelated invalidation)
  // never clobbers in-progress edits, and so switching agents re-seeds rather
  // than carrying one agent's edits onto another.
  const [manifestEditorSeededFor, setManifestEditorSeededFor] = useState<string | null>(null);
  const [manifestEditorFormState, setManifestEditorFormState] =
    useState<ManifestFormState>(emptyManifestForm);
  const [manifestEditorExtras, setManifestEditorExtras] =
    useState<ManifestExtras>(emptyManifestExtras);
  const [manifestEditorErrors, setManifestEditorErrors] = useState<Set<string>>(new Set());
  const [manifestEditorParseError, setManifestEditorParseError] = useState<string | null>(null);
  // The config tab is the manifest editor's only surface, so editing is live
  // exactly while that tab is selected. Every query the form needs hangs off
  // this one condition.
  const manifestEditorLive = mainTab === "config" && !!detailAgent;
  // The two second-level surfaces whose data is fetched at page level rather
  // than inside the component that shows it. Named rather than inlined because
  // each gates its queries: a panel fetched for every selected agent would turn
  // opening a page into five requests.
  const toolsGroupOpen = mainTab === "config" && configGroup === "tools";
  const historyTabOpen = mainTab === "info" && infoTab === "history";
  const addToast = useUIStore((s) => s.addToast);
  useCreateShortcut(() => setShowCreate(true));
  const templatesQuery = useAgentTemplates({
    enabled: showCreate && createMode === "template",
  });
  const localizedTemplates = useMemo(
    () =>
      (templatesQuery.data ?? []).map((template) => ({
        ...template,
        displayName: t(`agents.builtin.${template.name}.name`, { defaultValue: template.name }),
        displayDescription: t(`agents.builtin.${template.name}.description`, {
          defaultValue: template.description || template.name,
        }),
      })),
    [templatesQuery.data, t],
  );
  const selectedTemplate = useMemo(
    () => localizedTemplates.find((template) => template.name === templateName) ?? null,
    [localizedTemplates, templateName],
  );
  const spawnMutation = useSpawnAgent();
  const suspendMutation = useSuspendAgent();
  const resumeMutation = useResumeAgent();
  const patchAgentMutation = usePatchAgent();
  // #7749 review: the config tab's Save must not read the rename flow's
  // mutation state — a failed rename (duplicate name → 400) would render its
  // error inside the manifest editor and an in-flight rename would disable
  // that Save. Its own instance, like `ChannelsSection` has its own.
  const manifestPatchMutation = usePatchAgent();
  const cloneMutation = useCloneAgent();
  const resetSessionMutation = useResetAgentSession();
  const templateTomlMutation = useAgentTemplateToml();
  const qc = useQueryClient();

  // --- Visual identity of the agent in the drawer (#8339) ------------------
  const whoami = useWhoami();
  const canEditAppearance = canEditAgentIdentity(whoami.data?.role);

  const detailIdentity = (detailAgent as AgentView | null)?.identity;
  // Gated on "this agent has one" so an agent without an avatar costs no
  // request at all; `undefined` while loading or absent, which is what `Avatar`
  // wants — it falls back to the initials on its own.
  const detailAvatarSrc = useAgentAvatarUrl(detailAgent?.id ?? "", !!detailIdentity?.avatar_url);

  const rawDeleteMutation = useDeleteAgent();
  const handleDeleteSuccess = (agentId: string) => {
    if (detailAgent?.id === agentId) {
      setDetailAgent(null);
      setDetailDrawerOpen(false);
    } else {
      setDetailAgent(prev => prev);
    }
    addToast(t("agents.delete_success", { defaultValue: "Agent deleted" }), "success");
  };
  const handleDeleteError = (e: Error) =>
    addToast(
      e?.message || t("agents.delete_failed", { defaultValue: "Failed to delete agent" }),
      "error",
    );
  // Cancelling the agent's in-flight reads now lives in `useDeleteAgent`'s
  // `onMutate` (see `lib/mutations/agents.ts`), which is both the right layer
  // and the only place the cancel can be awaited before the DELETE is sent.
  const deleteMutation = {
    mutate: (agentId: string) =>
      rawDeleteMutation.mutate(agentId, {
        onSuccess: () => handleDeleteSuccess(agentId),
        onError: handleDeleteError,
      }),
    mutateAsync: (agentId: string) =>
      rawDeleteMutation.mutateAsync(agentId, {
        onSuccess: () => handleDeleteSuccess(agentId),
        onError: handleDeleteError,
      }),
  };

  function mergeHandFlag(agent: AgentDetail, fallback?: boolean) {
    return { ...agent, is_hand: agent.is_hand ?? fallback };
  }

  // The single-agent detail response omits lineage — only the list
  // endpoint includes it — so origin fields are carried over from the
  // list row or the previous detail state on refresh.
  function mergeOriginFields<T extends AgentDetail>(
    agent: T,
    origin?: Pick<AgentView, "parent_agent_id" | "parent" | "parent_unknown" | "children">,
  ): T {
    if (!origin) return agent;
    // `goToAgent` fabricates a stub row for an id the list does not hold, and
    // `selectAgent`'s catch branch builds one too. Neither carries lineage, so
    // overwriting with their `undefined`s would make the Origin panel report
    // "Root agent" for an agent whose parent was simply never fetched — which
    // is what `routes/agents/mod.rs:377` explicitly tells clients not to do
    // ("a client must not render the latter as a root agent"). Carry lineage
    // over only when the source actually has some.
    const carriesLineage =
      origin.parent_agent_id !== undefined ||
      origin.parent !== undefined ||
      origin.parent_unknown !== undefined ||
      origin.children !== undefined;
    if (!carriesLineage) return agent;
    return {
      ...agent,
      // `AgentItem` documents `parent` as the raw `AgentEntry` serde form that
      // endpoints serializing the struct directly emit, so accept either.
      parent_agent_id: origin.parent_agent_id ?? origin.parent,
      parent_unknown: origin.parent_unknown,
      children: origin.children,
    };
  }

  function closeDetailModal() {
    // Closing the drawer no longer deselects the agent — the inline detail
    // panel remains visible. Use deselectAgent() to fully exit the
    // selection (e.g. when an agent is deleted).
    setDetailDrawerOpen(false);
    setEditingName(false);
  }

  function startNameEdit() {
    setNameDraft(detailAgent?.name ?? "");
    setEditingName(true);
  }

  function cancelNameEdit() {
    setEditingName(false);
  }

  function saveName() {
    // Re-entrancy guard: Enter pressed twice in quick succession would
    // otherwise queue a second PATCH for the same name, which the kernel's
    // `update_name` rejects with `AgentAlreadyExists` (the name_index
    // entry was just claimed by the first call) — surfacing as a
    // misleading "Failed to rename" toast for the user. The Save button
    // already has the same disable check; mirror it here for keyboard
    // submits.
    if (patchAgentMutation.isPending) return;
    const trimmed = nameDraft.trim();
    if (!detailAgent || !trimmed || trimmed === detailAgent.name) {
      setEditingName(false);
      return;
    }
    // Capture the agent id we're renaming. If the user closes this modal
    // and opens a different agent before the mutation resolves, the
    // onSuccess handler must NOT smuggle this rename's name onto the new
    // agent's local state.
    const targetId = detailAgent.id;
    patchAgentMutation.mutate(
      { agentId: targetId, body: { name: trimmed } },
      {
        onSuccess: () => {
          // Optimistic local update so the header reflects the new name
          // before the agents query refetch lands. Gate on id so a stale
          // mutation completing after the user navigated to another
          // agent doesn't overwrite that other agent's name.
          setDetailAgent(prev =>
            prev?.id === targetId ? { ...prev, name: trimmed } : prev,
          );
          setEditingName(false);
          addToast(t("agents.rename_success", { defaultValue: "Agent renamed" }), "success");
        },
        onError: (e: Error) => {
          addToast(
            e?.message || t("agents.rename_failed", { defaultValue: "Failed to rename agent" }),
            "error",
          );
        },
      },
    );
  }

  async function refreshDetailAgent(agentId: string, fallback?: boolean) {
    try {
      await qc.invalidateQueries({ queryKey: agentQueries.detail(agentId).queryKey });
      const d = await qc.fetchQuery(agentQueries.detail(agentId));
      setDetailAgent(mergeOriginFields(mergeHandFlag(d, fallback), (detailAgent as AgentView) ?? undefined));
    } catch {
      // keep current state when refresh fails
    }
  }

  // Tool catalog, for the skill/tool finder in the create-agent form dialog
  // (#5049) and for the declared/allow/block tool fields the config tab's form
  // renders. The Tools Editor modal that used to own the per-agent copy is
  // gone: the manifest form is the one writer of those lists now.
  const toolsListQuery = useTools({
    enabled:
      (showCreate && createMode === "form") ||
      (!!detailAgent && toolsGroupOpen) ||
      manifestEditorLive,
  });

  // Share the snapshot query with OverviewPage — same cache key means React Query
  // deduplicates the poll when both pages are mounted, and agent counts on the
  // Overview tab stay in sync with this list automatically.
  const agentsQuery = useDashboardSnapshot();
  // Detail-panel data sources. Memory + audit are global lists filtered
  // client-side by agent id; cron is server-side filtered (its own
  // `enabled` flag gates the network request on `detailAgent?.id`).
  // TanStack Query dedupes / caches across pages so revisiting an agent
  // is free.
  // Per-agent KV memory (matches the design canvas's key/value/age rows).
  // The previous useMemorySearchOrList(\"\") query returned global proactive
  // memory, which is empty unless [proactive_memory] is enabled — so the
  // tab read empty even when the agent had KV pairs.
  const agentKvMemoryQuery = useAgentKvMemory(detailAgent?.id ?? "");
  // Per-agent KPI rollup (#4246) — replaces a global /api/sessions scan
  // that was capped by pagination and missed agents whose sessions
  // weren't in the latest N rows.
  const agentStatsQuery = useAgentStats(detailAgent?.id ?? "");
  // Cron jobs + per-agent triggers are now consumed inside
  // <AgentSchedulePanel/> (issue #4924). React-Query dedupes identical
  // queryKey subscriptions across components, so dropping the page-level
  // hooks does not double-fetch.
  // Per-agent recent turn events — backs the Logs tab. usage_events is
  // turn-level (model dispatch, latency, tokens, cost) — exactly what
  // the design's stderr-style log feed wants. The previous source
  // (global audit) only had admin lifecycle entries, leaving the tab
  // blank for almost every agent.
  // The model router's profile catalog backs the manifest form's
  // allowed_profiles finder — the capability the routing panel used to own
  // exclusively, ported when the panel died.
  const routerProfilesQuery = useModelRouterProfiles();
  const agentEventsQuery = useAgentEvents(detailAgent?.id ?? "", 30);
  const routerProfileCatalog = useMemo(
    () =>
      (routerProfilesQuery.data?.profiles ?? []).map((p) => ({
        name: p.name,
        description: [`${p.provider}/${p.model}`, p.cost_tier].join(" · "),
      })),
    [routerProfilesQuery.data],
  );
  // Per-agent MCP server assignment (#7713). The Tools & skills group is where
  // MCP grants are explained — now by the manifest form's own `mcp_servers`
  // section, which renders the pending list this query backs. A server with no
  // live connection contributes no tools, so it crosses nowhere else and would
  // otherwise be invisible on this page.
  const tabAgentMcpQuery = useAgentMcpServers(detailAgent?.id ?? "", {
    enabled: !!detailAgent && toolsGroupOpen,
  });

  // Manifest version history — fetched only when the History tab is active.
  const manifestHistoryQuery = useAgentManifestHistory(detailAgent?.id ?? "", {
    enabled: !!detailAgent && historyTabOpen,
  });

  // Per-agent session list — Conversation tab uses this directly. The
  // global /api/sessions used previously was paginated to 50, so the
  // agent's latest session was often not in the page.
  const agentSessionsQuery = useAgentSessions(detailAgent?.id ?? "");
  // Sourced from useAgentSessions (/api/agents/{id}/sessions) — NOT the
  // global useSessions() — to avoid the global endpoint's 50-row pagination
  // cap silently hiding this agent's newest session on busy systems. See
  // issue #4294 and lib/sessionSelector.ts for the regression test.
  const latestSessionForAgent = useMemo(
    () => pickLatestSessionId(agentSessionsQuery.data),
    [agentSessionsQuery.data],
  );
  const sessionDetailQuery = useSessionDetails(latestSessionForAgent ?? "");

  const formModelsQueryProvider = showCreate
    ? formState.model.provider
    : manifestEditorFormState.model.provider;
  const formModelsQuery = useModels(
    { provider: formModelsQueryProvider },
    {
      enabled:
        ((showCreate && createMode === "form") || manifestEditorLive) &&
        !!formModelsQueryProvider.trim(),
    },
  );

  const providersQuery = useProviders();

  // Global skill registry — used to cross-reference descriptions for the
  // names listed in the agent's `skills` allowlist (issue #4925) and to
  // seed the skill finder in the create dialog (issue #5049). The hook
  // is gated on either an open detail panel or an open form-mode create
  // dialog so the list isn't fetched at page load when neither is
  // active. `staleTime` from `useSkills` defaults to 30s, matching
  // SkillsPage, so opening multiple agents in quick succession reuses
  // the cache.
  const skillsQuery = useSkills({
    enabled: !!detailAgent || (showCreate && createMode === "form"),
  });
  // Raw manifest TOML for the full manifest editor (#7742) — only fetched
  // while that drawer is open for the currently selected agent.
  const agentManifestQuery = useAgentManifest(detailAgent?.id ?? "", {
    enabled: manifestEditorLive,
  });

  const configuredProviders = useMemo(
    // Suppression excluded as well as availability: a provider the operator
    // removed is absent from the Providers page, and offering it here would
    // let an agent be bound to something with no card, no badge and no way to
    // manage it.
    () => (providersQuery.data ?? []).filter(p => p.suppressed !== true && isProviderAvailable(p.auth_status)),
    [providersQuery.data],
  );

  // Form-mode option lists (only providers that have credentials configured).
  const formProviderOptions = useMemo(
    () => configuredProviders.map((p) => ({ name: p.id })),
    [configuredProviders],
  );
  const formModelOptions = useMemo(
    () =>
      (formModelsQuery.data?.models ?? []).map((m) => ({
        provider: m.provider,
        id: m.id,
        // Carried through so the editor's ladders stop where the endpoint does
        // and the over-limit advisory has something real to compare against.
        context_window: m.context_window,
        max_output_tokens: m.max_output_tokens,
        limits_known: m.limits_known,
      })),
    [formModelsQuery.data?.models],
  );

  // Catalogs for the create-dialog skill/tool finders (#5049). Stay
  // `undefined` until the underlying queries resolve so AgentManifestForm
  // falls back to its plain tag input instead of rendering an empty
  // combobox that misleads users into thinking "no skills are
  // available". An empty array, by contrast, IS a legitimate state
  // (registry returned zero rows) and is rendered as such.
  const skillCatalogForForm = useMemo<
    { name: string; description?: string }[] | undefined
  >(
    () =>
      skillsQuery.data
        ? skillsQuery.data.map((s) => ({ name: s.name, description: s.description }))
        : undefined,
    [skillsQuery.data],
  );
  const toolCatalogForForm = useMemo<
    { name: string; description?: string }[] | undefined
  >(
    () =>
      toolsListQuery.data
        ? toolsListQuery.data.map((tool: ToolDefinition) => ({
            name: tool.name,
            description: tool.description,
          }))
        : undefined,
    [toolsListQuery.data],
  );
  // Configured MCP servers catalog for the create-dialog finder (#5246).
  // The MCP servers field previously rendered as a free-text TagInput,
  // forcing users to remember server names exactly. Gate-fetch the
  // catalog only while the form-mode create dialog is open so we don't
  // hold an open `/api/mcp/servers` poll when the page first loads.
  // `refetchInterval: false` overrides the 30s default poll baked into
  // `useMcpServers` — the catalog only needs to be fresh on dialog open,
  // and reopening the dialog triggers a fresh fetch via the `enabled`
  // toggle anyway.
  const mcpServersQuery = useMcpServers({
    enabled: (showCreate && createMode === "form") || manifestEditorLive,
    refetchInterval: false,
  });
  const mcpCatalogForForm = useMemo<
    { name: string; description?: string }[] | undefined
  >(
    () =>
      mcpServersQuery.data
        ? mcpServersQuery.data.configured.map((s) => ({ name: s.name }))
        : undefined,
    [mcpServersQuery.data],
  );
  const serializedFormToml = useMemo(
    () => serializeManifestForm(formState, formExtras),
    [formState, formExtras],
  );
  const serializedFormMarkdown = useMemo(
    () => generateManifestMarkdown(formState, formExtras),
    [formState, formExtras],
  );
  const [previewTab, setPreviewTab] = useState<"toml" | "markdown">("toml");

  // Single close path for the create modal so the X button, the
  // Cancel button, and the onSuccess handler after spawn all clear the
  // same transient state. Template selection + custom name are cleared
  // here because they're per-attempt picks; form/TOML drafts persist so
  // users can reopen the modal and resume where they left off.
  const closeCreateModal = () => {
    setShowCreate(false);
    setFormErrors(new Set());
    setTomlParseError(null);
    setTemplateName("");
    setTemplateCustomName("");
    // Drop the incoming `template` param on the way out. Without this, pressing
    // Run on the same type a second time would navigate to a URL that already
    // equals the current one, the effect below would not re-run, and the drawer
    // would stay shut — the press would look like nothing happened.
    if (routeTemplate) {
      void navigate({ to: "/agents", search: {}, replace: true });
      // This drawer was opened by a Run press rather than by the Create button,
      // so hand the tab back to its default. Leaving it on Template would show
      // an empty picker with Create disabled the next time the drawer is opened
      // by hand. A tab the operator picked for themselves still persists.
      setCreateMode("form");
    }
    // Don't reset while a spawn is in flight — reset() flips isPending
    // back to false, and since the fetch isn't actually aborted the user
    // could reopen the modal and submit again before the first response
    // lands, producing a duplicate-spawn "already exists" error (the
    // exact bug #2741 was meant to fix). Once the original request
    // settles, isPending goes false on its own.
    if (!spawnMutation.isPending) {
      spawnMutation.reset();
    }
  };

  // Arriving from the agent-types page's Run button: the operator asked to
  // instantiate that type, so the create drawer opens on the template tab with
  // the type already selected. Run used to ask which *existing* agent to fork
  // instead, which answers a different question than the button asks.
  useEffect(() => {
    const seed = createDrawerSeed(routeTemplate);
    if (!seed) return;
    setShowCreate(true);
    setCreateMode(seed.createMode);
    setTemplateName(seed.templateName);
  }, [routeTemplate]);

  // Bidirectional Form ⇄ TOML sync. Going Form→TOML pushes the form's
  // serialized output into the textarea so advanced users can keep editing.
  // Going TOML→Form parses what's in the textarea, populating the form
  // and stashing unmapped fields ([thinking], [tools.*], etc.) in extras
  // so they survive a re-serialize.
  const switchCreateMode = (next: "form" | "template" | "toml") => {
    if (next === createMode) return;
    if (next === "form" && manifestToml.trim() && manifestToml !== serializedFormToml) {
      const parsed = parseManifestToml(manifestToml);
      if (!parsed.ok) {
        const parseMessage = parsed.message === "json_schema_unsafe_integer"
          ? t("agents.form.json_schema_unsafe_integer")
          : parsed.message;
        setTomlParseError(
          parsed.line !== undefined
            ? `Line ${parsed.line}:${parsed.column ?? 0} — ${parseMessage}`
            : parseMessage,
        );
        return;
      }
      setFormState(parsed.form);
      setFormExtras(parsed.extras);
      setTomlParseError(null);
    }
    if (next === "toml" && createMode === "form") {
      setManifestToml(serializedFormToml);
      setTomlParseError(null);
    }
    setCreateMode(next);
  };

  // Full manifest editor (#7742) — reset and seed. Distinct from the create
  // dialog's Form⇄TOML sync above: this is a single seed-once parse (the
  // editor's own TOML source is the server, not a sibling tab), not a
  // bidirectional textarea round-trip.
  //
  // Seeding is keyed on the agent, not on a surface being launched: the
  // sections render inside whichever group the operator is on, so the form has
  // to hold the selected agent's manifest for as long as the config tab is
  // selected.
  const manifestEditorAgentId = detailAgent?.id ?? null;

  // Clear the previous agent's parse the moment the selection changes.
  // Without this the tabs render one agent's values under another agent's
  // name until the manifest request lands — and, because the query is seeded
  // once, they would keep rendering them.
  useEffect(() => {
    setManifestEditorSeededFor(null);
    setManifestEditorErrors(new Set());
    setManifestEditorParseError(null);
    setManifestEditorFormState(emptyManifestForm());
    setManifestEditorExtras(emptyManifestExtras());
    // #7749 review: the query cache holds the TOML for `staleTime: 30_000`,
    // so reopening inside that window would serve a stale copy that the seed
    // effect below marks as seeded — and a save would then write the older
    // manifest over whatever changed on the server since (file-watcher
    // reload, another tab, POST /reload). Drop the entry so the enabled query
    // refetches and the editor only ever seeds from post-open data.
    if (manifestEditorLive && manifestEditorAgentId) {
      qc.removeQueries({ queryKey: agentQueries.manifest(manifestEditorAgentId).queryKey });
    }
  }, [manifestEditorAgentId, manifestEditorLive, qc]);

  useEffect(() => {
    if (!manifestEditorLive || !manifestEditorAgentId) return;
    if (manifestEditorSeededFor === manifestEditorAgentId) return;
    if (!agentManifestQuery.data) return;
    const parsed = parseManifestToml(agentManifestQuery.data);
    if (parsed.ok) {
      setManifestEditorFormState(parsed.form);
      setManifestEditorExtras(parsed.extras);
      setManifestEditorParseError(null);
    } else {
      setManifestEditorParseError(
        parsed.message === "json_schema_unsafe_integer"
          ? t("agents.form.json_schema_unsafe_integer")
          : parsed.message,
      );
    }
    setManifestEditorSeededFor(manifestEditorAgentId);
  }, [
    manifestEditorLive,
    manifestEditorAgentId,
    manifestEditorSeededFor,
    agentManifestQuery.data,
    t,
  ]);

  const saveManifestEditor = () => {
    if (!detailAgent) return;
    const errors = validateManifestForm(manifestEditorFormState);
    setManifestEditorErrors(new Set(errors));
    if (errors.length > 0) {
      // The offending field may live in a config group the operator is not
      // looking at, and with the sections grouped that is the common case
      // rather than an edge one — an error nobody can see is indistinguishable
      // from no error at all. This sends them to the config tab and to the
      // group that owns the first one; `Field` and the section's own `invalid`
      // flag then highlight it and force its section open, so the jump lands
      // on something that reads as an explanation rather than as a stray
      // navigation.
      const owningGroup = groupForFirstInvalidField(errors);
      if (owningGroup) {
        setMainTab("config");
        setConfigGroup(owningGroup);
      }
      return;
    }
    // The blocklist wins, so the overlap is resolved before the manifest is
    // written — what the form's conflict notice promises. The cleared entries
    // are also dropped from the editor's state, so the lists on screen match
    // the file that was just saved.
    const resolvedState = resolveToolListConflicts(manifestEditorFormState);
    if (resolvedState !== manifestEditorFormState) {
      setManifestEditorFormState(resolvedState);
    }
    const toml = serializeManifestForm(resolvedState, manifestEditorExtras);
    manifestPatchMutation.mutate(
      { agentId: detailAgent.id, body: { manifest_toml: toml } },
      {
        onSuccess: async () => {
          addToast(
            t("agents.detail.manifest_saved", { defaultValue: "Configuration saved" }),
            "success",
          );
          await refreshDetailAgent(detailAgent.id, detailAgent.is_hand);
        },
        onError: (e: Error) =>
          addToast(e.message || t("common.error", { defaultValue: "Error" }), "error"),
      },
    );
  };

  const agents = useMemo(() => agentsQuery.data?.agents ?? [], [agentsQuery.data?.agents]);
  const visibleAgents = useMemo(
    () => showHandAgents ? agents : agents.filter(a => !a.is_hand),
    [agents, showHandAgents],
  );
  // Counts for the filter chips so operators can see "5 running / 2
  // suspended" without running through the filter first.
  const agentCounts = useMemo(() => {
    const visible = visibleAgents;
    const running = visible.filter(a => (a.state || "").toLowerCase() === "running").length;
    const suspended = visible.filter(a => (a.state || "").toLowerCase() === "suspended").length;
    return { all: visible.length, running, suspended };
  }, [visibleAgents]);
  const filteredAgents = useMemo(() => visibleAgents
    .filter(a => {
      if (stateFilter === "all") return true;
      return (a.state || "").toLowerCase() === stateFilter;
    })
    .filter(a => a.name.toLowerCase().includes(search.toLowerCase()) || a.id.toLowerCase().includes(search.toLowerCase()))
    .sort((a, b) => {
      // Suspended always last regardless of primary sort — otherwise a
      // "sort by recent" view would bury running agents behind stale
      // suspended ones that happened to be touched recently.
      const aSusp = (a.state || "").toLowerCase() === "suspended" ? 1 : 0;
      const bSusp = (b.state || "").toLowerCase() === "suspended" ? 1 : 0;
      if (aSusp !== bSusp) return aSusp - bSusp;
      if (sortBy === "last_active") {
        const aT = a.last_active ? Date.parse(a.last_active) : 0;
        const bT = b.last_active ? Date.parse(b.last_active) : 0;
        return bT - aT; // most recent first
      }
      if (sortBy === "created_at") {
        const aT = a.created_at ? Date.parse(a.created_at) : 0;
        const bT = b.created_at ? Date.parse(b.created_at) : 0;
        return bT - aT; // newest first
      }
      return a.name.localeCompare(b.name);
    }), [visibleAgents, search, stateFilter, sortBy]);

  // handAgents is naturally empty while the showHandAgents toggle is off.
  const coreAgents = useMemo(() => filteredAgents.filter(a => !a.is_hand), [filteredAgents]);
  const handAgents = useMemo(() => filteredAgents.filter(a => a.is_hand), [filteredAgents]);

  const drawerDetailState = detailAgent ? ((detailAgent as AgentView).state || "").toLowerCase() : "";
  const isDetailDrawerSuspended = drawerDetailState === "suspended";
  const isDetailDrawerCrashed = drawerDetailState === "crashed";
  const drawerStatusColor = isDetailDrawerSuspended ? "bg-warning" : isDetailDrawerCrashed ? "bg-error" : "bg-success";
  const lockRename = !!detailAgent?.is_hand;

  const selectAgent = async (agent: AgentItem) => {
    // Selecting an agent lands on its live state, not on the group the
    // operator was editing in the previous one — that group's fields are the
    // previous agent's manifest.
    setMainTab("info");
    setInfoTab("logs");
    try {
      const d = await qc.fetchQuery(agentQueries.detail(agent.id));
      setDetailAgent(mergeOriginFields(mergeHandFlag(d, agent.is_hand), agent));
    } catch {
      setDetailAgent(mergeOriginFields({ name: agent.name, id: agent.id, is_hand: agent.is_hand } as AgentDetail, agent));
    }
  };

  /** Navigate the detail panel to a parent/child agent referenced by id.
   *  Falls back to a bare stub when the id isn't in the current (paginated)
   *  list — `selectAgent` still fetches the real detail from its own id. */
  const goToAgent = (id: string) => {
    const found = agents.find(a => a.id === id);
    void selectAgent(found ?? ({ id, name: id, is_hand: false } as AgentItem));
  };

  // Auto-select the first agent on desktop so the detail panel isn't blank
  // on first paint. Skipped on mobile because the detail view is a full-
  // screen overlay there — auto-opening it would block access to the list.
  useEffect(() => {
    if (detailAgent) return;
    if (filteredAgents.length === 0) return;
    // 1000px matches the `--breakpoint-lg` override in index.css (#4873).
    if (typeof window.matchMedia === "function" && !window.matchMedia("(min-width: 1000px)").matches) return;
    void selectAgent(filteredAgents[0]);
    // selectAgent is recreated each render; depending on filteredAgents+detailAgent is sufficient.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filteredAgents, detailAgent]);

  const renderAgentRow = (agent: AgentItem) => {
    const isSelected = detailAgent?.id === agent.id;
    // Row-embedded stats from /api/agents (single grouped SQL pass). The
    // canonical envelope always ships these fields; see `enrich_agent_json`.
    const sessions24h = typeof agent.sessions_24h === "number" ? agent.sessions_24h : 0;
    const cost24h = typeof agent.cost_24h === "number" ? agent.cost_24h : 0;
    const stats = { sessions24h, cost24h };
    const stateLower = (agent.state || "").toLowerCase();
    return (
      <button
        key={agent.id}
        type="button"
        onClick={() => void selectAgent(agent)}
        className={`w-full text-left px-3.5 py-2.5 border-l-2 border-b border-border-subtle/40 transition-colors cursor-pointer ${
          isSelected
            ? "border-l-brand bg-brand/5"
            : "border-l-transparent bg-transparent hover:bg-main/40"
        } ${stateLower === "suspended" ? "opacity-70" : ""}`}
      >
        <div className="flex items-center gap-2 min-w-0">
          {/* `<div>`, not `<span>`: `AgentAvatar` renders `Avatar`, which is a
              `<div>`, and a span cannot legally contain one. */}
          <div className="relative shrink-0">
            <AgentAvatar
              agentId={agent.id}
              avatarUrl={agent.identity?.avatar_url}
              emoji={agent.identity?.emoji}
              fallback={agent.name}
              size="sm"
            />
            {/* State pinned to the avatar's corner, the way the detail drawer
                below places it, instead of the pill this row used to carry —
                that pill held nothing but the dot. Placement is the same; the
                colour is not, and was not before either: this row reads
                `getStatusVariant`, the drawer its own three-way map. */}
            <span
              className={`absolute -bottom-0.5 -right-0.5 w-2.5 h-2.5 rounded-full ${dotColors[getStatusVariant(agent.state)]} border-2 border-surface`}
              role="img"
              aria-label={agent.state || "idle"}
            />
          </div>
          <span className="font-mono text-[13px] truncate flex-1 min-w-0 text-text-main">
            {t(`agents.builtin.${agent.name}.name`, { defaultValue: agent.name })}
          </span>
          {agent.is_hand && (
            <span className="shrink-0 text-[9px] font-bold px-1.5 py-px rounded bg-brand/10 text-brand">
              {t("agents.hand_badge", { defaultValue: "HAND" })}
            </span>
          )}
          {!!agent.children?.length && (
            <span className="shrink-0 text-[10.5px] text-text-dim/80">
              ({t("agents.children_count", { count: agent.children.length })})
            </span>
          )}
          <span className="font-mono text-[10.5px] text-text-dim/80 shrink-0 tabular-nums">
            {agent.last_active ? formatRelativeTime(agent.last_active) : "—"}
          </span>
        </div>
        <div className="font-mono text-[10.5px] text-text-dim flex items-center gap-2 pl-10 mt-1">
          <span className="truncate min-w-0">{agent.model_name || agent.model_provider || "—"}</span>
          <span className="text-text-dim/60">·</span>
          <span className="truncate min-w-0">
            {agent.schedule || t("agents.schedule_manual", { defaultValue: "manual" })}
          </span>
          {agent.source_template && (
            <>
              <span className="text-text-dim/60">·</span>
              <span className="truncate min-w-0">
                {t("agents.origin_template", { name: agent.source_template })}
              </span>
            </>
          )}
          <span className="ml-auto shrink-0 tabular-nums">
            {stats.sessions24h} · ${stats.cost24h.toFixed(2)}
          </span>
        </div>
      </button>
    );
  };

  // Inline detail panel — replaces the old card-grid + drawer mix.
  //
  // Two tabs, and the split is by what the operator is doing. "logs & info"
  // reads the agent: a brief, then logs, memory, prompts and manifest history.
  // "config" writes it: every manifest section, grouped, one save.
  //
  // The two tab lists are derived from `CONFIG_GROUP_IDS` and `INFO_TABS`
  // rather than written out here, so a group added to the map cannot end up
  // without a tab to reach it.
  const mainTabs: Array<AgentTabDef<AgentMainTab>> = [
    { id: "info", label: t("agents.tab.logs_info", { defaultValue: "logs & info" }), Icon: FileText },
    { id: "config", label: t("agents.tab.config", { defaultValue: "config" }), Icon: Settings },
  ];
  const infoTabs: Array<AgentTabDef<InfoTab>> = [
    { id: "logs", label: t("agents.info.logs", { defaultValue: "Logs" }), Icon: FileText },
    { id: "memory", label: t("agents.info.memory", { defaultValue: "Memory" }), Icon: Database },
    { id: "prompts", label: t("agents.info.prompts", { defaultValue: "Prompts & experiments" }), Icon: FlaskConical },
    { id: "history", label: t("agents.info.history", { defaultValue: "History" }), Icon: History },
  ];
  // The group tabs, in the order the map declares. The icon per group is a
  // property of the group, not of the map — `CONFIG_GROUPS` is about which
  // fields live where, and a presentation choice there would be an icon in
  // the middle of a data structure.
  const GROUP_ICONS: Record<ConfigGroupId, typeof Bot> = {
    general: Bot,
    model: Route,
    // `KeyRound` rather than the `Shield` that Limits uses: this group is about
    // what the agent is allowed to do, and a second shield beside "Limits &
    // cost" would read as the same subject twice.
    permissions: KeyRound,
    tools: Wrench,
    memory: Database,
    limits: Shield,
    channels: Radio,
    planning: Clock,
    conversation: Sparkles,
  };
  // The label comes from the id, with no `defaultValue`: the English text
  // for each group lives in the locale files like every other label, and a
  // second copy here would be one more place to keep in step. The guard in
  // `AgentsPage.test.tsx` fails if an id has no label in any locale, which is
  // what keeps a missing one from rendering as a raw key.
  const configTabs: Array<AgentTabDef<ConfigGroupId>> = CONFIG_GROUP_IDS.map(
    (id) => ({
      id,
      label: t(`agents.group.${id}`),
      Icon: GROUP_ICONS[id],
    }),
  );

  const renderDetailPanel = (agent: AgentDetail) => {
    const detailState = ((agent as AgentView).state || "").toLowerCase();
    const isSuspended = detailState === "suspended";
    const isCrashed = detailState === "crashed";
    return (
      <Card padding="none" className="surface-lit overflow-hidden flex flex-col min-h-0 lg:min-h-[640px] h-full lg:h-auto">
        {/* Header */}
        <div className="px-3 sm:px-5 pt-3 sm:pt-4 pb-3 border-b border-border-subtle">
          <div className="flex items-center gap-2 sm:gap-3">
            {/* Mobile-only "back to list" affordance — closes the detail
                overlay without deselecting state for lg+. */}
            <button
              type="button"
              onClick={() => setDetailAgent(null)}
              className="lg:hidden -ml-1 p-1.5 rounded-md text-text-dim hover:text-text-main hover:bg-main/40 shrink-0"
              aria-label={t("common.back", { defaultValue: "Back" })}
            >
              <X className="w-4 h-4" />
            </button>
            <div className="w-9 h-9 rounded-lg bg-brand/10 border border-brand/30 grid place-items-center text-brand shrink-0">
              <Bot className="w-[18px] h-[18px]" />
            </div>
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2 min-w-0">
                <h2 className="font-mono font-semibold text-base truncate text-text-main">
                  {t(`agents.builtin.${agent.name}.name`, { defaultValue: agent.name })}
                </h2>
                <Badge variant={getStatusVariant((agent as AgentView).state)} dot className="shrink-0">
                  {(agent as AgentView).state
                    ? t(`common.${((agent as AgentView).state || "").toLowerCase()}`, { defaultValue: (agent as AgentView).state })
                    : t("common.idle")}
                </Badge>
              </div>
              <p className="font-mono text-[11.5px] text-text-dim/80 truncate mt-0.5">
                {truncateId(agent.id)}
                {agent.model?.model || (agent as AgentView).model_name
                  ? ` · ${agent.model?.model || (agent as AgentView).model_name}`
                  : ""}
                {(agent as AgentView).profile ? ` · ${(agent as AgentView).profile}` : ""}
              </p>
            </div>
            {/* Action cluster — labels collapse on mobile so the row keeps
                three icon-buttons + back arrow on a 390px viewport. */}
            <div className="flex items-center gap-1 sm:gap-1.5 shrink-0">
              {isSuspended ? (
                <Button
                  variant="ghost"
                  size="sm"
                  leftIcon={<Play className="w-3.5 h-3.5" />}
                  aria-label={t("agents.resume", { defaultValue: "Resume" })}
                  title={t("agents.resume", { defaultValue: "Resume" })}
                  onClick={async () => {
                    try { await resumeMutation.mutateAsync(agent.id); } catch (e) {
                      addToast(toastErr(e, t("agents.resume_failed", { defaultValue: "Failed to resume agent" })), "error");
                    }
                  }}
                >
                  <span className="hidden sm:inline">{t("agents.resume", { defaultValue: "Resume" })}</span>
                </Button>
              ) : (
                <Button
                  variant="ghost"
                  size="sm"
                  leftIcon={<Pause className="w-3.5 h-3.5" />}
                  aria-label={t("agents.suspend", { defaultValue: "Pause" })}
                  title={t("agents.suspend", { defaultValue: "Pause" })}
                  onClick={async () => {
                    try { await suspendMutation.mutateAsync(agent.id); } catch (e) {
                      addToast(toastErr(e, t("agents.suspend_failed", { defaultValue: "Failed to suspend agent" })), "error");
                    }
                  }}
                >
                  <span className="hidden sm:inline">{t("agents.suspend", { defaultValue: "Pause" })}</span>
                </Button>
              )}
              {/* Quick Run (#6699) — a one-off ephemeral worker on this
                  agent's budget. Anchored here rather than on the agent-types
                  row because the parent that is billed, and whose
                  `[resources]` quota is the ceiling, is the agent, not a type.
                  Hidden for hands, which cannot be a parent. */}
              {!agent.is_hand && (
                <Button
                  variant="ghost"
                  size="sm"
                  leftIcon={<Zap className="w-3.5 h-3.5" />}
                  aria-label={t("agents.quick_run")}
                  title={t("agents.quick_run")}
                  onClick={() => setQuickRunParent(agent.id)}
                >
                  <span className="hidden sm:inline">{t("agents.quick_run")}</span>
                </Button>
              )}
              <Button
                variant="secondary"
                size="sm"
                leftIcon={<MessageCircle className="w-3.5 h-3.5" />}
                aria-label={t("common.interact", { defaultValue: "Chat" })}
                title={t("common.interact", { defaultValue: "Chat" })}
                onClick={() => navigate({ to: "/chat", search: { agentId: agent.id } })}
              >
                <span className="hidden sm:inline">{t("common.interact", { defaultValue: "Chat" })}</span>
              </Button>
              {/* Details — the avatar editor, the agent's read-only lineage
                  and its lifecycle actions (reset, delete, chat). Not
                  configuration: that lives on the config tab, and a second
                  surface that writes the same manifest is what the reform
                  removed. */}
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setDetailDrawerOpen(true)}
                title={t("common.details", { defaultValue: "Details" })}
                aria-label={t("common.details", { defaultValue: "Details" })}
              >
                <MoreHorizontal className="w-4 h-4" />
              </Button>
            </div>
          </div>

          {/* The two main tabs. The KPI tiles and the token footprint used to
              sit here, above every tab, which put a reading surface on top of
              a writing one — they are the first thing "logs & info" shows. */}
          <AgentTabBar
            tabs={mainTabs}
            active={mainTab}
            onSelect={setMainTab}
            ariaLabel={t("agents.tabs.main", { defaultValue: "Agent view" })}
            className="mt-4 -mb-3"
          />
        </div>

        {/* Tab content */}
        <div className="flex-1 overflow-y-auto px-3 sm:px-5 py-3 sm:py-4">
          {mainTab === "info"
            ? renderInfoTab(agent, isCrashed)
            : renderConfigTab(agent)}
        </div>
      </Card>
    );
  };

  // ---------- "logs & info" — the brief, then four read-only surfaces.
  //
  // The brief is not a sub-tab: it is what the operator came for, and hiding
  // the state, the cost and the tail of the conversation behind one more click
  // is how the token footprint ended up in a drawer.
  const renderInfoTab = (agent: AgentDetail, isCrashed: boolean) => {
    const sessionData = sessionDetailQuery.data as
      | { messages?: AgentBriefMessage[] }
      | undefined;
    const detailCaps = (agent as AgentView).capabilities;
    const toolsCount = Array.isArray(detailCaps?.tools) ? detailCaps.tools.length : 0;

    const skillNames: string[] = (
      Array.isArray((agent as AgentView).skills)
        ? ((agent as AgentView).skills as string[])
        : Array.isArray((agent as AgentView).capabilities?.skills)
          ? (((agent as AgentView).capabilities!.skills) as string[])
          : []
    )
      .slice()
      .sort();
    const toolsMeta = skillNames.length > 0
      ? skillNames.slice(0, 3).join(" · ")
      : toolsCount > 0
        ? `${toolsCount} configured`
        : "—";

    return (
      <div className="flex flex-col gap-4">
        {isCrashed && (
          <EmptyState
            title={t("agents.detail.crashed_title", {
              defaultValue: "{{name}} is in error state",
              name: agent.name,
            })}
            icon={<X className="h-6 w-6 text-error" />}
            action={
              <Button variant="primary" size="sm" leftIcon={<RotateCcw className="h-3.5 w-3.5" />} onClick={async () => {
                try { await resumeMutation.mutateAsync(agent.id); } catch (e) {
                  addToast(toastErr(e, t("agents.resume_failed", { defaultValue: "Failed to resume agent" })), "error");
                }
              }}>
                {t("agents.resume", { defaultValue: "Resume" })}
              </Button>
            }
          />
        )}
        <AgentBrief
          agent={agent as AgentView}
          stats={agentStatsQuery.data}
          events={agentEventsQuery.data ?? []}
          messages={sessionData?.messages ?? []}
          conversationLoading={sessionDetailQuery.isLoading}
          hasSession={!!latestSessionForAgent}
          toolsCount={toolsCount}
          toolsMeta={toolsMeta}
          onOpenChat={() => navigate({ to: "/chat", search: { agentId: agent.id } })}
        />
        <AgentTabBar
          tabs={infoTabs}
          active={infoTab}
          onSelect={setInfoTab}
          ariaLabel={t("agents.tabs.info", { defaultValue: "Agent information" })}
          variant="pill"
        />
        <div>
          {infoTab === "logs" && renderLogsTab(agent)}
          {infoTab === "memory" && renderMemoryTab(agent)}
          {infoTab === "prompts" && <PromptsExperimentsPanel agentId={agent.id} />}
          {infoTab === "history" && renderHistoryTab(agent)}
        </div>
      </div>
    );
  };

  // ---------- "config" — every manifest section, grouped, one save.
  //
  // The group is the tab. Inside it, the live panels that own a grant over
  // their own endpoint (cron, channels) come first — they are what is actually
  // in effect right now — and the manifest form follows, because the form is
  // the one writer of the file those grants end up in. The Skills and Tools
  // panels that used to sit above it are gone: they were a second writer of
  // `capabilities_tools` and `skills`, and their fields live in the form's
  // `skills` / `tools` / `mcp_servers` sections now.
  const renderConfigTab = (agent: AgentDetail) => {
    if (manifestEditorParseError) {
      return (
        <p className="text-xs text-error">
          {t("agents.form.toml_parse_error", { msg: manifestEditorParseError })}
        </p>
      );
    }
    if (agentManifestQuery.isError) {
      return (
        <p className="text-xs text-error">
          {t("agents.detail.manifest_load_failed", {
            defaultValue: "Failed to load the current configuration.",
          })}
        </p>
      );
    }
    if (agentManifestQuery.isLoading) {
      return (
        <p className="text-xs text-text-dim flex items-center gap-2">
          <Loader2 className="w-3.5 h-3.5 animate-spin" />
          {t("common.loading", { defaultValue: "Loading..." })}
        </p>
      );
    }
    return (
      <div className="flex flex-col gap-4">
        {/* The switch governs every section's folded half, and Save commits the
            whole manifest — so both belong at the top of the tab, not at the
            bottom of a group the operator may not be in. */}
        <div className="flex items-center justify-between gap-3 flex-wrap">
          <label className="inline-flex items-center gap-2 cursor-pointer select-none">
            <input
              type="checkbox"
              checked={advancedMode}
              onChange={(e) => setAdvancedMode(e.target.checked)}
              className="h-3.5 w-3.5 accent-brand"
              aria-label={t("agents.config.advanced_mode", { defaultValue: "Advanced mode" })}
            />
            <span className="text-xs font-semibold text-text-dim">
              {t("agents.config.advanced_mode", { defaultValue: "Advanced mode" })}
            </span>
            <span className="text-[11px] text-text-dim/70 hidden sm:inline">
              {t("agents.config.advanced_hint", {
                defaultValue: "show every field, not just the everyday ones",
              })}
            </span>
          </label>
          <Button
            variant="primary"
            size="sm"
            onClick={saveManifestEditor}
            disabled={manifestPatchMutation.isPending}
          >
            {manifestPatchMutation.isPending
              ? t("common.saving", { defaultValue: "Saving..." })
              : t("common.save", { defaultValue: "Save" })}
          </Button>
        </div>
        <AgentTabBar
          tabs={configTabs}
          active={configGroup}
          onSelect={setConfigGroup}
          ariaLabel={t("agents.tabs.config_groups", { defaultValue: "Configuration groups" })}
          // Pill, like the info sub-tabs: these are one level below the main
          // tabs and the same shape says so without a second legend.
          variant="pill"
        />
        <div className="flex flex-col gap-4">
          {configGroup === "channels" && <ChannelsSection agentId={agent.id} />}
          {configGroup === "planning" && <AgentSchedulePanel agent={agent} />}
          <AgentManifestForm
            value={manifestEditorFormState}
            onChange={setManifestEditorFormState}
            providers={formProviderOptions}
            models={formModelOptions}
            invalidFields={manifestEditorErrors}
            extras={manifestEditorExtras}
            skillCatalog={skillCatalogForForm}
            toolCatalog={toolCatalogForForm}
            mcpCatalog={mcpCatalogForForm}
            // The declared-but-unconnected crossing the removed Tools tab used
            // to be alone in showing; the form renders it in `mcp_servers`.
            mcpPending={tabAgentMcpQuery.data?.pending ?? []}
            routerProfileCatalog={routerProfileCatalog}
            routerProfilesEnabled={routerProfilesQuery.data?.enabled}
            // Identity is decided by the panel header's rename control; a
            // second editable Name field here would be a second answer to the
            // same question.
            nameField="readonly"
            sections={CONFIG_GROUPS[configGroup] as ManifestSectionId[]}
            advanced={advancedMode}
          />
        </div>
      </div>
    );
  };

  // ---------- Memory tab — per-agent KV row layout per design canvas
  const renderMemoryTab = (agent: AgentDetail) => {
    const kv = agentKvMemoryQuery.data ?? [];

    // The kv_store backs both real KV (`user.preferences.tone` →
    // `"concise"`) and the proactive-memory cache (key = `memory:<uuid>`,
    // value = the full MemoryItem JSON). Render those two cases
    // differently so the proactive entries don't dump 600-char JSON
    // blobs into the row.
    type View = { key: string; value: string; ageIso?: string };
    const projected: View[] = kv.map((r) => {
      const value = r.value as unknown;
      if (
        typeof r.key === "string" &&
        r.key.startsWith("memory:") &&
        value &&
        typeof value === "object" &&
        !Array.isArray(value)
      ) {
        const obj = value as Record<string, unknown>;
        const content = typeof obj.content === "string" ? obj.content : "";
        const category = typeof obj.category === "string" ? obj.category : "memory";
        const createdAt = typeof obj.created_at === "string" ? obj.created_at : undefined;
        return { key: category, value: content || "—", ageIso: createdAt };
      }
      // Plain KV: show value as a string. Avoid JSON.stringify wrapping
      // strings with extra quotes.
      const valueStr = typeof value === "string"
        ? value
        : value == null
          ? "—"
          : safeStringify(value);
      return {
        key: r.key,
        value: valueStr,
        ageIso: r.created_at,
      };
    });
    const rows = projected.slice(0, 8);
    return (
      <div className="flex flex-col gap-3">
        <div className="flex items-center justify-between">
          <div className="text-[11px] uppercase font-semibold tracking-[0.08em] text-text-dim">
            {t("agents.detail.memory_label", { defaultValue: "Memory · sqlite" })} · {kv.length}
          </div>
          <Button
            variant="ghost"
            size="sm"
            leftIcon={<Database className="h-3.5 w-3.5" />}
            onClick={() => navigate({ to: "/memory", search: { agent: agent.id } })}
          >
            {t("agents.detail.open_memory", { defaultValue: "Open" })}
          </Button>
        </div>
        {agentKvMemoryQuery.isLoading ? (
          <div className="text-[12px] text-text-dim italic">{t("common.loading", { defaultValue: "Loading..." })}</div>
        ) : rows.length === 0 ? (
          <div className="rounded-md border border-border-subtle bg-main/40 p-4 text-[12px] text-text-dim italic">
            {t("agents.detail.no_memory", { defaultValue: "No memory entries yet for this agent." })}
          </div>
        ) : (
          <div className="flex flex-col gap-1.5">
            {rows.map((r, i) => (
              <div
                key={`${r.key}-${i}`}
                className="flex flex-col sm:flex-row sm:items-center gap-1 sm:gap-2.5 px-3 py-2 rounded-md border border-border-subtle bg-main/40"
              >
                <div className="flex items-center justify-between gap-2 sm:contents">
                  <span className="font-mono text-[12px] text-brand sm:min-w-[180px] truncate sm:shrink-0 min-w-0">{r.key}</span>
                  <span className="font-mono text-[10.5px] text-text-dim/70 sm:order-3 sm:shrink-0 tabular-nums shrink-0">
                    {r.ageIso ? formatRelativeTime(r.ageIso) : "—"}
                  </span>
                </div>
                <span className="font-mono text-[12px] text-text-dim sm:flex-1 min-w-0 truncate sm:order-2" title={r.value}>
                  {r.value}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
    );
  };

  // ---------- Schedule tab — runtime registries only (issue #4924).
  // AgentSchedulePanel owns the CRUD endpoints (POST/PATCH/DELETE
  // /api/triggers and /api/cron/jobs). The manifest's [schedule] — mode,
  // cron, conditions — is edited by the manifest form the same tab hosts,
  // not by a second surface here. The synthetic "Last 14 runs" bar chart was
  // a placeholder (no real per-fire telemetry endpoint yet) and was dropped
  // in favour of real editing affordances — restore it once a per-agent
  // run-history feed exists.
  // ---------- Logs tab — terminal-style turn feed per design canvas
  // Sourced from /api/agents/{id}/events (usage_events) so each row is
  // a real LLM turn — model / latency / tokens / cost — instead of the
  // global audit ledger, which is mostly admin lifecycle entries.
  const renderLogsTab = (_agent: AgentDetail) => {
    const events = agentEventsQuery.data ?? [];
    const fmtTime = (s?: string): string => {
      if (!s) return "—";
      try {
        const d = new Date(s);
        const hh = String(d.getHours()).padStart(2, "0");
        const mm = String(d.getMinutes()).padStart(2, "0");
        const ss = String(d.getSeconds()).padStart(2, "0");
        const ms = String(d.getMilliseconds()).padStart(3, "0");
        return `${hh}:${mm}:${ss}.${ms}`;
      } catch {
        return s;
      }
    };
    // Token shorthand (1.2k tok) — keeps the line fitting the design.
    const fmtTokens = (n: number): string =>
      n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
    const formatLine = (e: typeof events[number]): string =>
      `turn · ${e.model} · in=${fmtTokens(e.input_tokens)} out=${fmtTokens(e.output_tokens)} · ${e.latency_ms}ms · $${e.cost_usd.toFixed(4)}`;
    return (
      <div className="flex flex-col gap-3">
        <div className="flex items-center justify-between">
          <div className="text-[11px] uppercase font-semibold tracking-[0.08em] text-text-dim">
            {t("agents.detail.events_tail", { defaultValue: "events · tail" })} · {events.length}
          </div>
          <Button
            variant="ghost"
            size="sm"
            leftIcon={<Copy className="h-3.5 w-3.5" />}
            onClick={() => {
              const text = events
                .map((e) => `${fmtTime(e.timestamp)} INFO ${e.provider || "—"} ${formatLine(e)}`)
                .join("\n");
              // The toast has to follow the result: `copyToClipboard` resolves to `false` on a non-secure origin rather than throwing, and a "Copied" toast for a write that never happened is the same silent failure as no toast at all.
              void copyToClipboard(text).then((ok) =>
                ok
                  ? addToast(t("common.copied", { defaultValue: "Copied" }), "success")
                  : addToast(t("common.copy_failed", { defaultValue: "Copy failed" }), "error"),
              );
            }}
          >
            {t("common.copy", { defaultValue: "Copy" })}
          </Button>
        </div>
        {agentEventsQuery.isLoading ? (
          <div className="text-[12px] text-text-dim italic">{t("common.loading", { defaultValue: "Loading..." })}</div>
        ) : events.length === 0 ? (
          <div className="rounded-md border border-border-subtle bg-main/40 p-4 text-[12px] text-text-dim italic">
            {t("agents.detail.no_logs", { defaultValue: "No turns recorded yet for this agent." })}
          </div>
        ) : (
          <div
            className="rounded-md border border-border-subtle p-3 font-mono text-[11.5px] leading-[1.6] max-h-60 overflow-auto -mx-3 sm:mx-0"
            style={{ background: "rgba(2,6,23,0.6)" }}
          >
            {events.map((e, i) => (
              <div key={`${e.timestamp}-${i}`} className="flex gap-2.5 min-w-max sm:min-w-0">
                <span className="text-text-dim/60 shrink-0">{fmtTime(e.timestamp)}</span>
                <span className="text-success w-12 shrink-0">INFO</span>
                <span className="text-accent w-24 shrink-0 truncate">{e.provider || "agent"}</span>
                <span className="text-text-dim min-w-0 truncate" title={formatLine(e)}>
                  {formatLine(e)}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
    );
  };

  // ---------- History tab — manifest version timeline
  const renderHistoryTab = (_agent: AgentDetail) => {
    const versions = manifestHistoryQuery.data ?? [];
    return (
      <div className="flex flex-col gap-3">
        <div className="text-[11px] uppercase font-semibold tracking-[0.08em] text-text-dim">
          {t("agents.detail.manifest_history", { defaultValue: "Manifest history" })} · {versions.length}
        </div>
        {manifestHistoryQuery.isLoading ? (
          <div className="text-[12px] text-text-dim italic">{t("common.loading", { defaultValue: "Loading..." })}</div>
        ) : manifestHistoryQuery.isError ? (
          <div className="rounded-md border border-red-500/30 bg-red-500/10 p-4 text-[12px] text-red-500">
            {t("agents.detail.manifest_history_load_err", { defaultValue: "Failed to load manifest history." })}
          </div>
        ) : versions.length === 0 ? (
          <div className="rounded-md border border-border-subtle bg-main/40 p-4 text-[12px] text-text-dim italic">
            {t("agents.detail.no_history", { defaultValue: "No config changes recorded yet." })}
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            {versions.map((v) => (
              <details
                key={v.id}
                className="rounded-md border border-border-subtle bg-main/40 group"
              >
                <summary className="px-3 py-2 cursor-pointer text-[12px] flex items-center gap-2 select-none">
                  <History className="w-3.5 h-3.5 text-text-dim shrink-0" />
                  <span className="font-medium">{formatSqliteDateTime(v.timestamp)}</span>
                  <span className="text-text-dim">· {v.change_source}</span>
                </summary>
                <pre className="px-3 pb-3 text-[11px] font-mono leading-[1.6] max-h-60 overflow-auto whitespace-pre-wrap break-all text-text-dim">
                  {v.manifest_toml}
                </pre>
              </details>
            ))}
          </div>
        )}
      </div>
    );
  };

  return (
    <div className="flex flex-col gap-4 sm:gap-6 transition-colors duration-300">
      <div className="flex flex-col sm:flex-row justify-between items-start sm:items-end gap-3">
        <PageHeader
          badge={t("common.kernel_runtime")}
          title={t("agents.title")}
          subtitle={t("agents.subtitle")}
          isFetching={agentsQuery.isFetching}
          onRefresh={() => void agentsQuery.refetch()}
          icon={<Users className="h-4 w-4" />}
          helpText={t("agents.help")}
        />
        <Button variant="primary" onClick={() => setShowCreate(true)} className="shrink-0" title={t("agents.create_agent") + " (n)"}>
          <Plus className="w-4 h-4" />
          <span>{t("agents.create_agent")}</span>
          <kbd className="hidden sm:inline-flex h-5 min-w-[20px] items-center justify-center rounded border border-white/30 bg-white/10 px-1 text-[9px] font-mono font-semibold">n</kbd>
        </Button>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-[360px_1fr] gap-4 lg:min-h-[640px]">
        {/* Left list panel — search + filter pills + sort + scroll body.
            On mobile the list owns the viewport; the detail panel is
            promoted to a full-screen overlay (see below) so we never
            stack two scrollers on a 390px phone. */}
        <Card
          padding="none"
          className={`surface-lit overflow-hidden flex flex-col h-[calc(100vh-200px)] min-h-[480px] ${detailAgent ? "hidden lg:flex" : "flex"}`}
        >
          <div className="px-3 pt-3 pb-2.5 border-b border-border-subtle flex flex-col gap-2 flex-shrink-0">
            <Input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder={t("common.search")}
              leftIcon={<Search className="h-4 w-4" />}
              data-shortcut-search
            />
            <div className="flex flex-wrap gap-1.5 items-center">
              {(["all", "running", "suspended"] as const).map((key) => {
                const isActive = stateFilter === key;
                const count = agentCounts[key];
                const label = t(`agents.filter_${key}`, {
                  defaultValue: key === "all" ? "All" : key === "running" ? "Running" : "Suspended",
                });
                return (
                  <button
                    key={key}
                    onClick={() => setStateFilter(key)}
                    className={`inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[10.5px] font-semibold transition-colors ${
                      isActive
                        ? "border-brand/30 bg-brand/10 text-brand"
                        : "border-border-subtle bg-surface text-text-dim hover:border-brand/20 hover:text-brand"
                    }`}
                  >
                    <span>{label}</span>
                    <span
                      className={`inline-flex items-center justify-center rounded-full px-1 min-w-[16px] h-[14px] text-[9px] font-mono ${
                        isActive ? "bg-brand/20" : "bg-main"
                      }`}
                    >
                      {count}
                    </span>
                  </button>
                );
              })}
              <button
                onClick={() => setShowHandAgents((value) => !value)}
                aria-pressed={showHandAgents}
                className={`inline-flex items-center rounded-full border px-2 py-0.5 text-[10.5px] font-semibold transition-colors ${
                  showHandAgents
                    ? "border-brand/30 bg-brand/10 text-brand"
                    : "border-border-subtle bg-surface text-text-dim hover:border-brand/20 hover:text-brand"
                }`}
              >
                {t("agents.show_hand_agents_short", { defaultValue: "Hand" })}
              </button>
              <select
                value={sortBy}
                onChange={(e) => setSortBy(e.target.value as typeof sortBy)}
                className="ml-auto rounded-full border border-border-subtle bg-surface px-2 py-0.5 text-[10.5px] font-semibold text-text-dim outline-none focus:border-brand hover:border-brand/20 cursor-pointer"
                aria-label={t("common.sort_by", { defaultValue: "Sort by" })}
              >
                <option value="name">{t("common.sort_name", { defaultValue: "Name" })}</option>
                <option value="last_active">{t("common.sort_last_active", { defaultValue: "Active" })}</option>
                <option value="created_at">{t("common.sort_created", { defaultValue: "Created" })}</option>
              </select>
            </div>
          </div>
          <div className="flex-1 overflow-y-auto">
            {agentsQuery.isLoading ? (
              <div className="p-3 flex flex-col gap-2">
                {[1, 2, 3, 4, 5].map((i) => <CardSkeleton key={i} />)}
              </div>
            ) : filteredAgents.length === 0 ? (
              search || stateFilter !== "all" || showHandAgents ? (
                <EmptyState
                  title={t("agents.no_matching")}
                  icon={<Search className="h-6 w-6" />}
                  action={
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => {
                        setSearch("");
                        setStateFilter("all");
                        setShowHandAgents(false);
                      }}
                    >
                      {t("common.clear_filters", { defaultValue: "Clear filters" })}
                    </Button>
                  }
                />
              ) : (
                <EmptyState title={t("common.no_data")} icon={<Users className="h-6 w-6" />} />
              )
            ) : (
              <div className="flex flex-col">
                {coreAgents.length > 0 && (
                  <>
                    <div className="px-3.5 py-1.5 text-[10px] uppercase font-semibold tracking-[0.08em] text-text-dim bg-main/40 border-b border-border-subtle/40">
                      {t("agents.core_agents")} ({coreAgents.length})
                    </div>
                    {coreAgents.map((agent) => renderAgentRow(agent))}
                  </>
                )}
                {handAgents.length > 0 && (
                  <>
                    <div className="px-3.5 py-1.5 text-[10px] uppercase font-semibold tracking-[0.08em] text-text-dim bg-main/40 border-b border-border-subtle/40">
                      {t("agents.hands")} ({handAgents.length})
                    </div>
                    {handAgents.map((agent) => renderAgentRow(agent))}
                  </>
                )}
              </div>
            )}
          </div>
        </Card>

        {/* Right detail panel — header, then the two main tabs. The KPI
            tiles and the token footprint live in the brief inside "logs &
            info", not above both tabs.
            Mobile: rendered as a fixed full-viewport overlay above the
            list (top inset 0, bottom inset 14 reserves the global tab
            bar's ~56px so it never gets covered). lg+: collapses back
            into the master-detail grid. */}
        {detailAgent ? (
          <div className="fixed inset-x-0 top-0 bottom-[calc(56px+env(safe-area-inset-bottom))] z-30 bg-surface lg:static lg:inset-auto lg:bottom-auto lg:z-auto lg:bg-transparent overflow-hidden flex flex-col">
            {renderDetailPanel(detailAgent)}
          </div>
        ) : (
          // Placeholder is desktop-only — on mobile the list fills the
          // viewport when no agent is selected, so this empty-state would
          // just be wasted vertical space.
          <Card padding="lg" className="surface-lit hidden lg:grid place-items-center text-center min-h-[480px]">
            <div className="max-w-xs">
              <div className="w-12 h-12 mx-auto rounded-xl bg-brand/10 border border-brand/30 grid place-items-center text-brand mb-3">
                <Bot className="w-6 h-6" />
              </div>
              <h3 className="text-sm font-semibold text-text-main mb-1">
                {t("agents.select_an_agent", { defaultValue: "Select an agent" })}
              </h3>
              <p className="text-xs text-text-dim">
                {t("agents.select_an_agent_hint", {
                  defaultValue: "Choose an agent on the left to inspect its sessions, memory, skills, and live logs.",
                })}
              </p>
            </div>
          </Card>
        )}
      </div>
      {/* Agent Detail Drawer. Right-side inspector pattern (Linear / Figma):
          the agents list stays interactive while the drawer is open, so
          clicking another agent in the list updates the drawer's content
          in place — no close-then-reopen needed. Sticky header / footer
          keep identity and primary actions pinned while the inspectable
          sections scroll in the middle. */}
      {detailAgent && detailDrawerOpen && (
        <DrawerPanel
          isOpen
          onClose={closeDetailModal}
          size="xl"
          hideCloseButton
        >
            <div className="px-6 py-4 border-b border-border-subtle sticky top-0 bg-surface z-10">
              <div className="flex items-start justify-between gap-3">
                <div className="flex items-start gap-3 min-w-0 flex-1">
                  <div className="relative shrink-0">
                    <Avatar
                      fallback={detailAgent.name}
                      size="lg"
                      src={detailAvatarSrc}
                      emoji={detailIdentity?.emoji}
                    />
                    <span
                      className={`absolute -bottom-0.5 -right-0.5 w-3 h-3 rounded-full ${drawerStatusColor} border-2 border-surface ${!isDetailDrawerSuspended && !isDetailDrawerCrashed ? "animate-pulse" : ""}`}
                      role="img"
                      aria-label={
                        isDetailDrawerSuspended
                          ? t("agents.status_suspended", { defaultValue: "Agent suspended" })
                          : isDetailDrawerCrashed
                          ? t("agents.status_crashed", { defaultValue: "Agent crashed" })
                          : t("agents.status_active", { defaultValue: "Agent active" })
                      }
                    />
                  </div>
                  <div className="min-w-0 flex-1">
                    {editingName ? (
                      <div className="flex items-center gap-2">
                        <input
                          type="text"
                          autoFocus
                          value={nameDraft}
                          onChange={e => setNameDraft(e.target.value)}
                          onKeyDown={e => {
                            // `isComposing` guard: in CJK IMEs Enter
                            // confirms the candidate (pinyin → hanzi);
                            // submitting on it would hijack composition.
                            if (e.key === "Enter" && !e.nativeEvent.isComposing) {
                              saveName();
                            } else if (e.key === "Escape") {
                              // stopPropagation so Escape cancels the inline
                              // edit only — the Modal's window Escape listener
                              // would otherwise close the whole modal too.
                              e.stopPropagation();
                              cancelNameEdit();
                            }
                          }}
                          className="px-2 py-1 rounded-lg border border-brand bg-main text-base font-bold outline-none focus:ring-2 focus:ring-brand/30 min-w-0 flex-1"
                          aria-label={t("agents.edit_name", { defaultValue: "Agent name" })}
                          maxLength={64}
                        />
                        <button
                          onClick={saveName}
                          disabled={patchAgentMutation.isPending || !nameDraft.trim() || nameDraft.trim() === detailAgent.name}
                          className="px-3 py-1 rounded-lg text-xs font-semibold bg-brand text-white hover:bg-brand/90 disabled:opacity-50 disabled:cursor-not-allowed shrink-0"
                        >
                          {patchAgentMutation.isPending ? t("common.saving") : t("common.save")}
                        </button>
                        <button
                          onClick={cancelNameEdit}
                          className="px-3 py-1 rounded-lg text-xs font-semibold bg-main hover:bg-main/80 text-text-dim border border-border-subtle shrink-0"
                        >
                          {t("common.cancel")}
                        </button>
                      </div>
                    ) : (
                      <button
                        type="button"
                        onClick={lockRename ? undefined : startNameEdit}
                        disabled={lockRename}
                        className={`group inline-flex items-center gap-2 max-w-full ${lockRename ? "cursor-default" : "cursor-text hover:text-brand transition-colors"}`}
                        title={lockRename
                          ? t("agents.rename_hand_disabled", { defaultValue: "Hand-managed agents cannot be renamed" })
                          : t("agents.rename_hint", { defaultValue: "Click to rename" })}
                      >
                        <h3 className="text-base font-bold truncate">
                          {t(`agents.builtin.${detailAgent.name}.name`, { defaultValue: detailAgent.name })}
                        </h3>
                        {!lockRename && (
                          <Pencil className="w-3.5 h-3.5 text-text-dim opacity-0 group-hover:opacity-100 transition-opacity shrink-0" />
                        )}
                      </button>
                    )}
                    {(detailAgent as AgentView).description && (
                      <p className="text-xs text-text-dim mt-1 leading-relaxed">{(detailAgent as AgentView).description}</p>
                    )}
                    <div className="flex items-center gap-2 mt-1.5 flex-wrap">
                      <span className="text-[11px] text-text-dim/70 font-mono">{truncateId(detailAgent.id, 16)}</span>
                      {detailAgent.is_hand && <Badge variant="info">{t("agents.hand_badge", { defaultValue: "HAND" })}</Badge>}
                      <Badge variant={isDetailDrawerSuspended ? "warning" : isDetailDrawerCrashed ? "error" : "success"} dot>
                        {(detailAgent as AgentView).state ? t(`common.${drawerDetailState}`, { defaultValue: (detailAgent as AgentView).state }) : t("common.running")}
                      </Badge>
                    </div>
                  </div>
                </div>
                <button onClick={closeDetailModal} className="p-2 rounded-lg hover:bg-main transition-colors shrink-0" aria-label={t("common.close", { defaultValue: "Close" })}>
                  <X className="w-4 h-4" />
                </button>
              </div>
            </div>
            {/* Body — scrollable inspectable sections. */}
            <div className="px-6 py-5 space-y-5">

              {/* Appearance — the emoji and the avatar image (#8339).
                  Its own component, and exported, for the reason the file header
                  gives for the brief: `AgentsPage` has ~20 hooks and no render
                  harness, so anything that has to be tested has to be reachable
                  without mounting the page. */}
              {canEditAppearance && (
                <AgentAppearanceSection
                  agentId={detailAgent.id}
                  identity={detailIdentity}
                  onChanged={() => { void refreshDetailAgent(detailAgent.id); }}
                />
              )}

              {/* Origin */}
              <section>
                <h4 className="text-sm font-semibold flex items-center gap-2 mb-2">
                  <GitBranch className="w-3.5 h-3.5 text-brand" />
                  {t("agents.origin", { defaultValue: "Origin" })}
                </h4>
                <div className="rounded-lg bg-main border border-border-subtle p-4 space-y-2">
                  <DetailRow label={t("agents.parent", { defaultValue: "Parent Agent" })}>
                    {(() => {
                      const view = detailAgent as AgentView;
                      const parentId = view.parent_agent_id ?? view.parent;
                      // `enrich_agent_json` emits all three lineage fields
                      // together, so none of them present means lineage was
                      // never fetched for this agent — distinct from "it has
                      // no parent", and it must not be shown as a root agent.
                      if (
                        parentId === undefined &&
                        view.parent_unknown === undefined &&
                        view.children === undefined
                      ) {
                        return (
                          <span className="text-text-dim">
                            {t("agents.origin_unloaded")}
                          </span>
                        );
                      }
                      if (parentId) {
                        const parent = agents.find(a => a.id === parentId);
                        const label = parent
                          ? t(`agents.builtin.${parent.name}.name`, { defaultValue: parent.name })
                          : truncateId(parentId, 16);
                        return (
                          <button
                            type="button"
                            onClick={() => goToAgent(parentId)}
                            className="font-mono text-brand hover:underline"
                          >
                            {label}
                          </button>
                        );
                      }
                      if (view.parent_unknown) {
                        return (
                          <span className="text-text-dim">
                            {t("agents.origin_unknown")}
                          </span>
                        );
                      }
                      return (
                        <span className="text-text-dim">
                          {t("agents.origin_root", { defaultValue: "Root agent" })}
                        </span>
                      );
                    })()}
                  </DetailRow>
                  <DetailRow label={t("agents.children", { defaultValue: "Children" })}>
                    {(detailAgent as AgentView).children?.length ? (
                      <div className="flex flex-wrap justify-end gap-1.5">
                        {(detailAgent as AgentView).children!.map(childId => {
                          const child = agents.find(a => a.id === childId);
                          const label = child
                            ? t(`agents.builtin.${child.name}.name`, { defaultValue: child.name })
                            : truncateId(childId, 16);
                          return (
                            <button
                              key={childId}
                              type="button"
                              onClick={() => goToAgent(childId)}
                              className="font-mono text-xs px-1.5 py-0.5 rounded bg-brand/10 text-brand hover:bg-brand/20"
                            >
                              {label}
                            </button>
                          );
                        })}
                      </div>
                    ) : (
                      <span className="text-text-dim">{t("common.none")}</span>
                    )}
                  </DetailRow>
                </div>
              </section>

              {/* Capabilities */}
              {detailAgent.capabilities && (
                <section>
                  <h4 className="text-sm font-semibold mb-2 flex items-center gap-2">
                    <Wrench className="w-3.5 h-3.5 text-success" />
                    {t("agents.capabilities")}
                  </h4>
                  <div className="flex flex-wrap gap-2">
                    {detailAgent.capabilities.tools && (
                      <button
                        type="button"
                        className="inline-flex"
                        aria-label={t("agents.tools_edit_aria", { defaultValue: "Edit tools" })}
                        title={t("agents.tools_edit_aria", { defaultValue: "Edit tools" })}
                        onClick={(e: React.MouseEvent) => {
                          e.stopPropagation();
                          // The badge used to open the Tools Editor modal — a
                          // second writer of `capabilities_tools`. It is a
                          // pointer now, not a surface: close the drawer and
                          // land on the one editor that owns the list, with
                          // Advanced mode on because the declared, allow and
                          // block fields fold behind it.
                          closeDetailModal();
                          setMainTab("config");
                          setConfigGroup("permissions");
                          setAdvancedMode(true);
                        }}
                      >
                        <Badge variant="brand" dot className="hover:bg-brand/20 transition-colors">
                          {`${t("agents.tools_cap")} ✎`}
                        </Badge>
                      </button>
                    )}
                    {detailAgent.capabilities.network && <Badge variant="brand" dot>{t("agents.network")}</Badge>}
                  </div>
                </section>
              )}

              {/* Skills */}
              {detailAgent.skills && detailAgent.skills.length > 0 && (
                <section>
                  <h4 className="text-sm font-semibold mb-2">{t("agents.skills")}</h4>
                  <div className="flex flex-wrap gap-1.5">
                    {/* Sorted alphabetically (#4940) — matches the Skills tab. */}
                    {detailAgent.skills.slice().sort().map((s: string) => (
                      <Badge key={s} variant="default">{s}</Badge>
                    ))}
                  </div>
                </section>
              )}

              {/* Tags */}
              {detailAgent.tags && detailAgent.tags.length > 0 && (
                <section>
                  <h4 className="text-sm font-semibold mb-2">{t("agents.tags")}</h4>
                  <div className="flex flex-wrap gap-1.5">
                    {detailAgent.tags.map((tag: string, i: number) => (
                      <span
                        key={i}
                        className="text-xs px-2.5 py-1 rounded-md bg-main border border-border-subtle text-text-dim"
                      >
                        {tag}
                      </span>
                    ))}
                  </div>
                </section>
              )}

              {/* Mode */}
              {detailAgent.mode && (
                <section className="flex items-center gap-3 rounded-lg bg-main border border-border-subtle px-4 py-3">
                  <Shield className="w-4 h-4 text-warning shrink-0" />
                  <span className="text-sm font-semibold flex-1">{t("agents.mode")}</span>
                  <Badge variant="warning">{detailAgent.mode}</Badge>
                </section>
              )}

              {/* Thinking / Extended Reasoning */}
              {detailAgent.thinking && (
                <section>
                  <h4 className="text-sm font-semibold mb-2 flex items-center gap-2">
                    <Brain className="w-3.5 h-3.5 text-purple-500" />
                    {t("agents.thinking")}
                  </h4>
                  <div className="rounded-lg bg-main border border-border-subtle p-4 space-y-2">
                    <DetailRow label={t("agents.thinking_enabled")}>
                      <Badge variant={(detailAgent.thinking.budget_tokens ?? 0) > 0 ? "success" : "default"}>
                        {(detailAgent.thinking.budget_tokens ?? 0) > 0 ? t("common.yes") : t("common.no")}
                      </Badge>
                    </DetailRow>
                    <DetailRow label={t("agents.budget_tokens")}>
                      <span className="font-mono">{formatNumber(detailAgent.thinking.budget_tokens)}</span>
                    </DetailRow>
                    <DetailRow label={t("agents.stream_thinking")}>
                      <Badge variant={detailAgent.thinking.stream_thinking ? "brand" : "default"}>
                        {detailAgent.thinking.stream_thinking ? t("common.yes") : t("common.no")}
                      </Badge>
                    </DetailRow>
                    <p className="text-xs text-text-dim flex items-center gap-1.5 pt-1">
                      <Zap className="w-3 h-3" />
                      {t("agents.thinking_hint")}
                    </p>
                  </div>
                </section>
              )}
            </div>

            {/* Footer — sticky, primary + secondary actions reachable on long specs. */}
            <div className="sticky bottom-0 px-6 py-4 border-t border-border-subtle bg-surface space-y-2.5">
              <Button
                variant="primary"
                size="md"
                className="w-full"
                onClick={() => { closeDetailModal(); navigate({ to: "/chat", search: { agentId: detailAgent.id } }); }}
              >
                <MessageCircle className="w-4 h-4 mr-2" />
                {t("common.interact")}
              </Button>

              <div className="flex flex-wrap gap-2">
                {isDetailDrawerSuspended ? (
                  <Button
                    variant="secondary"
                    size="sm"
                    className="flex-1 min-w-[88px]"
                    onClick={async () => {
                      try {
                        await resumeMutation.mutateAsync(detailAgent.id);
                        await refreshDetailAgent(detailAgent.id, detailAgent.is_hand);
                      } catch (err) {
                        addToast(toastErr(err, t("agents.resume_failed", { defaultValue: "Failed to resume agent" })), "error");
                      }
                    }}
                  >
                    <Play className="w-3.5 h-3.5 mr-1.5" />
                    {t("agents.resume")}
                  </Button>
                ) : (
                  <Button
                    variant="secondary"
                    size="sm"
                    className="flex-1 min-w-[88px]"
                    onClick={async () => {
                      try {
                        await suspendMutation.mutateAsync(detailAgent.id);
                        await refreshDetailAgent(detailAgent.id, detailAgent.is_hand);
                      } catch (err) {
                        addToast(toastErr(err, t("agents.suspend_failed", { defaultValue: "Failed to suspend agent" })), "error");
                      }
                    }}
                  >
                    <Pause className="w-3.5 h-3.5 mr-1.5" />
                    {t("agents.suspend")}
                  </Button>
                )}
                <Button
                  variant="secondary"
                  size="sm"
                  className="flex-1 min-w-[88px]"
                  onClick={() => {
                    setCloneNameDraft(`${detailAgent.name}-copy`);
                    setCloneIncludeSkills(true);
                    setCloneIncludeTools(true);
                    setCloneDialog({ agentId: detailAgent.id, sourceName: detailAgent.name });
                  }}
                >
                  <Copy className="w-3.5 h-3.5 mr-1.5" />
                  {t("agents.clone")}
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  className="flex-1 min-w-[88px]"
                  onClick={() =>
                    setConfirmDialog({
                      title: t("agents.reset_title", { defaultValue: "Reset session?" }),
                      message: t("agents.reset_confirm"),
                      onConfirm: async () => {
                        await resetSessionMutation.mutateAsync(detailAgent.id);
                        await refreshDetailAgent(detailAgent.id, detailAgent.is_hand);
                      },
                    })
                  }
                >
                  <RotateCcw className="w-3.5 h-3.5 mr-1.5" />
                  {t("agents.reset")}
                </Button>
                {!detailAgent.is_hand && (
                  <Button
                    variant="secondary"
                    size="sm"
                    className="flex-1 min-w-[88px] text-error/80 hover:text-error"
                    onClick={() =>
                      setConfirmDialog({
                        title: t("agents.delete_title", { defaultValue: "Delete agent?" }),
                        message: t("agents.delete_confirm", { name: detailAgent.name }),
                        tone: "destructive",
                        onConfirm: async () => {
                          await deleteMutation.mutateAsync(detailAgent.id);
                        },
                      })
                    }
                  >
                    <Trash2 className="w-3.5 h-3.5 mr-1.5" />
                    {t("common.delete")}
                  </Button>
                )}
              </div>

            </div>
          </DrawerPanel>
      )}

      {/* Create Agent Modal */}
      <DrawerPanel
        isOpen={showCreate}
        onClose={closeCreateModal}
        title={t("agents.create_agent")}
        size="2xl"
      >
        <div className="p-5 space-y-4">
          {/* Mode tabs — switching between Form and TOML round-trips the
              manifest in both directions. We only re-parse when content
              actually differs, so re-clicking the same tab is a no-op. */}
          <div className="flex gap-2">
            <button onClick={() => switchCreateMode("form")}
              className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${createMode === "form" ? "bg-brand text-white" : "bg-main text-text-dim"}`}>
              {t("agents.from_form")}
            </button>
            <button onClick={() => switchCreateMode("template")}
              className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${createMode === "template" ? "bg-brand text-white" : "bg-main text-text-dim"}`}>
              {t("agents.from_template")}
            </button>
            <button onClick={() => switchCreateMode("toml")}
              className={`px-3 py-1.5 rounded-lg text-xs font-bold transition-colors ${createMode === "toml" ? "bg-brand text-white" : "bg-main text-text-dim"}`}>
              {t("agents.from_toml")}
            </button>
          </div>
          {tomlParseError && (
            // The error is set when leaving TOML→Form fails, so the user
            // is bounced back to TOML; the message must show on the TOML
            // tab too, otherwise the rejected switch is invisible.
            <p className="text-xs text-error">
              {t("agents.form.toml_parse_error", { msg: tomlParseError })}
            </p>
          )}

          {createMode === "form" ? (
            <div className="grid grid-cols-1 lg:grid-cols-[1fr_360px] gap-4 max-h-[60vh] overflow-y-auto pr-1">
              <AgentManifestForm
                value={formState}
                onChange={setFormState}
                providers={formProviderOptions}
                models={formModelOptions}
                invalidFields={formErrors}
                extras={formExtras}
                skillCatalog={skillCatalogForForm}
                toolCatalog={toolCatalogForForm}
                mcpCatalog={mcpCatalogForForm}
                routerProfileCatalog={routerProfileCatalog}
                routerProfilesEnabled={routerProfilesQuery.data?.enabled}
              />
              <div className="space-y-2">
                <div className="flex items-center justify-between gap-2">
                  <div className="flex gap-1">
                    <button
                      type="button"
                      onClick={() => setPreviewTab("toml")}
                      className={`text-[10px] font-bold uppercase px-2 py-1 rounded ${
                        previewTab === "toml"
                          ? "bg-brand text-white"
                          : "text-text-dim hover:text-text"
                      }`}
                    >
                      {t("agents.form.preview_toml")}
                    </button>
                    <button
                      type="button"
                      onClick={() => setPreviewTab("markdown")}
                      className={`text-[10px] font-bold uppercase px-2 py-1 rounded ${
                        previewTab === "markdown"
                          ? "bg-brand text-white"
                          : "text-text-dim hover:text-text"
                      }`}
                    >
                      {t("agents.form.preview_markdown")}
                    </button>
                  </div>
                  <div className="flex gap-2 items-center">
                    <button
                      type="button"
                      onClick={() => {
                        const text =
                          previewTab === "toml" ? serializedFormToml : serializedFormMarkdown;
                        void copyToClipboard(text).then((ok) =>
                          ok
                            ? addToast(t("agents.form.copied"), "success")
                            : addToast(
                                t("common.copy_failed", { defaultValue: "Copy failed" }),
                                "error",
                              ),
                        );
                      }}
                      className="text-[10px] font-bold text-text-dim hover:text-brand"
                      title={t("agents.form.copy")}
                    >
                      <Copy className="w-3.5 h-3.5" />
                    </button>
                    {previewTab === "toml" && (
                      <button
                        type="button"
                        onClick={() => switchCreateMode("toml")}
                        className="text-[10px] font-bold text-brand hover:underline"
                      >
                        {t("agents.form.switch_to_toml")}
                      </button>
                    )}
                  </div>
                </div>
                <pre className="rounded-xl border border-border-subtle bg-main px-3 py-2 text-[11px] font-mono text-text-dim overflow-auto max-h-[55vh] whitespace-pre-wrap break-all">
                  {previewTab === "toml" ? serializedFormToml : serializedFormMarkdown}
                </pre>
              </div>
            </div>
          ) : createMode === "template" ? (
            <div className="space-y-3">
              <div>
                <label className="text-[10px] font-bold text-text-dim uppercase">{t("agents.template_name")}</label>
                <select value={templateName}
                  onChange={e => setTemplateName(e.target.value)}
                  className="mt-1 w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand">
                  <option value="">{t("agents.template_placeholder")}</option>
                  {localizedTemplates.map(tmpl => (
                    <option key={tmpl.name} value={tmpl.name}>{tmpl.displayName}</option>
                  ))}
                </select>
                {selectedTemplate && (
                  <div className="mt-2 rounded-xl border border-border-subtle/60 bg-surface/60 px-3 py-2">
                    <p className="text-xs font-bold text-text">{selectedTemplate.displayName}</p>
                    <p className="mt-1 text-[11px] leading-relaxed text-text-dim">{selectedTemplate.displayDescription}</p>
                  </div>
                )}
              </div>
              <div>
                <label className="text-[10px] font-bold text-text-dim uppercase">
                  {t("agents.template_custom_name", { defaultValue: "Agent Name (optional)" })}
                </label>
                <input
                  type="text"
                  value={templateCustomName}
                  onChange={e => setTemplateCustomName(e.target.value)}
                  placeholder={
                    selectedTemplate?.name ??
                    t("agents.template_custom_name_placeholder", {
                      defaultValue: "Leave blank to use template default",
                    })
                  }
                  className="mt-1 w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-sm outline-none focus:border-brand"
                />
                <p className="text-[10px] text-text-dim mt-1">
                  {t("agents.template_custom_name_hint", {
                    defaultValue: "Override the template's default name so you can run multiple agents from the same template.",
                  })}
                </p>
              </div>
              <button
                type="button"
                disabled={!templateName || templateTomlLoading}
                  onClick={async () => {
                    if (!templateName) return;
                    setTemplateTomlLoading(true);
                    try {
                      const toml = await templateTomlMutation.mutateAsync(templateName);
                    // Carry the user's custom name across when dropping into
                    // TOML mode — otherwise the input they just typed gets
                    // silently discarded and the template's original name wins.
                    const customName = templateCustomName.trim();
                    const patched = customName
                      ? toml.replace(
                          /^name\s*=\s*(?:"[^"]*"|'[^']*')/m,
                          `name = "${customName.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`,
                        )
                      : toml;
                    setManifestToml(patched);
                    setCreateMode("toml");
                  } catch {
                    addToast(
                      t("agents.loading_template_toml_failed", {
                        defaultValue: "Failed to load template TOML",
                      }),
                      "error",
                    );
                  } finally {
                    setTemplateTomlLoading(false);
                  }
                }}
                className="text-[10px] font-bold text-brand hover:underline disabled:text-text-dim disabled:no-underline disabled:cursor-not-allowed"
              >
                {templateTomlLoading ? (
                  <span className="inline-flex items-center gap-1">
                    <Loader2 className="w-3 h-3 animate-spin" />
                    {t("agents.loading_template_toml", { defaultValue: "Loading template…" })}
                  </span>
                ) : (
                  t("agents.edit_template_toml", {
                    defaultValue: "Edit TOML for advanced customization →",
                  })
                )}
              </button>
            </div>
          ) : (
            <div>
              <label className="text-[10px] font-bold text-text-dim uppercase">{t("agents.manifest_toml")}</label>
              <textarea value={manifestToml} onChange={e => {
                  setManifestToml(e.target.value);
                  // Clear stale parse error so the user gets fresh feedback
                  // on their next switch attempt instead of seeing a message
                  // that may already be addressed.
                  if (tomlParseError) setTomlParseError(null);
                }}
                placeholder={'[agent]\nname = "my-agent"\n\n[model]\nprovider = "openai"\nmodel = "gpt-4o"\n\n[thinking]\nbudget_tokens = 10000\nstream_thinking = false'}
                rows={12}
                className="mt-1 w-full rounded-xl border border-border-subtle bg-main px-3 py-2 text-xs font-mono outline-none focus:border-brand resize-none" />
              <p className="text-[9px] text-text-dim/50 mt-1 flex items-center gap-1">
                <Brain className="w-3 h-3" />
                {t("agents.thinking_toml_hint")}
              </p>
            </div>
          )}

          {spawnMutation.error && (
            <p className="text-xs text-error">{toastErr(spawnMutation.error, String(spawnMutation.error))}</p>
          )}

          <div className="flex gap-2 pt-2">
            <Button variant="primary" className="flex-1"
              onClick={() => {
                const onSuccess = () => {
                  addToast(
                    t("agents.agent_created", { defaultValue: "Agent created" }),
                    "success",
                  );
                  closeCreateModal();
                };
                const onError = (e: Error) => {
                  addToast(
                    e?.message ||
                      t("agents.create_failed", { defaultValue: "Failed to create agent" }),
                    "error",
                  );
                };
                if (createMode === "form") {
                  // Names already preserved from `[workspaces]` entries the
                  // form can't render (mount-based declarations) — without
                  // this a form row can collide with one of them and the
                  // duplicate key only surfaces as an opaque server-side
                  // TOML parse error instead of the inline message below.
                  // The tool lists are resolved the same way the config tab's
                  // Save resolves them, so the conflict notice's promise holds
                  // on both save paths (see `resolveToolListConflicts`).
                  const resolvedForm = resolveToolListConflicts(formState);
                  if (resolvedForm !== formState) setFormState(resolvedForm);
                  const errors = validateManifestForm(
                    resolvedForm,
                    preservedWorkspaceNamesFromExtras(formExtras),
                  );
                  setFormErrors(new Set(errors));
                  if (errors.length > 0) return;
                  spawnMutation.mutate(
                    { manifest_toml: serializeManifestForm(resolvedForm, formExtras) },
                    { onSuccess, onError },
                  );
                  return;
                }
                const customName = templateCustomName.trim();
                spawnMutation.mutate(
                  createMode === "template"
                    ? { template: templateName, ...(customName ? { name: customName } : {}) }
                    : { manifest_toml: manifestToml },
                  { onSuccess, onError },
                );
              }}
              disabled={
                spawnMutation.isPending ||
                templateTomlLoading ||
                (createMode === "form"
                  ? !formState.name.trim() ||
                    !formState.model.provider.trim() ||
                    !formState.model.model.trim()
                  : createMode === "template"
                    ? !templateName.trim()
                    : !manifestToml.trim())
              }>
              {spawnMutation.isPending ? <Loader2 className="w-4 h-4 animate-spin mr-1" /> : <Plus className="w-4 h-4 mr-1" />}
              {t("agents.create_agent")}
            </Button>
            <Button variant="secondary" onClick={closeCreateModal}>{t("common.cancel")}</Button>
          </div>
        </div>
      </DrawerPanel>

      {/* Quick Run Modal (#6699) — one ephemeral worker, gone when it ends. */}
      {quickRunParent !== null && (
        <QuickRunModal
          initialParent={quickRunParent}
          onClose={() => setQuickRunParent(null)}
        />
      )}
      <ConfirmDialog
        isOpen={confirmDialog !== null}
        title={confirmDialog?.title ?? ""}
        message={confirmDialog?.message ?? ""}
        tone={confirmDialog?.tone}
        onConfirm={() => confirmDialog?.onConfirm()}
        onClose={() => setConfirmDialog(null)}
      />

      {/* Clone Agent Modal (#6566) — collects the required `new_name`. */}
      <Modal
        isOpen={cloneDialog !== null}
        onClose={() => setCloneDialog(null)}
        title={t("agents.clone_title", { defaultValue: "Clone agent" })}
        size="sm"
      >
        <form
          className="p-6 space-y-4"
          onSubmit={async (e) => {
            e.preventDefault();
            if (!cloneDialog) return;
            const newName = cloneNameDraft.trim();
            if (!newName) return;
            try {
              const result = await cloneMutation.mutateAsync({
                agentId: cloneDialog.agentId,
                payload: {
                  new_name: newName,
                  include_skills: cloneIncludeSkills,
                  include_tools: cloneIncludeTools,
                },
              });
              const notice = cloneResultNotice(result);
              if (notice.partial) {
                addToast(t("agents.clone_partial", {
                  defaultValue: "Agent cloned with incomplete initialization: {{warnings}}",
                  warnings: notice.warnings,
                }), "info");
              } else {
                addToast(t("agents.clone_succeeded", { defaultValue: "Agent cloned" }), "success");
              }
              setCloneDialog(null);
            } catch (err) {
              addToast(toastErr(err, t("agents.clone_failed", { defaultValue: "Failed to clone agent" })), "error");
            }
          }}
        >
          <p className="text-[11px] text-text-dim/70">
            {t("agents.clone_desc", {
              defaultValue: "Copies {{name}}'s manifest into a new agent. The name must be unique.",
              name: cloneDialog?.sourceName ?? "",
            })}
          </p>
          <div className="space-y-1.5">
            <label
              htmlFor="clone-agent-name"
              className="block text-[10px] font-black text-text-dim uppercase tracking-widest"
            >
              {t("agents.clone_name_label", { defaultValue: "New agent name" })}
            </label>
            <Input
              id="clone-agent-name"
              value={cloneNameDraft}
              onChange={(e) => setCloneNameDraft(e.target.value)}
              placeholder={t("agents.clone_name_placeholder", { defaultValue: "my-agent-copy" })}
              maxLength={256}
              autoFocus
            />
          </div>
          <div className="space-y-2">
            <label className="flex items-center gap-2 text-[11px] text-text-dim">
              <input
                type="checkbox"
                checked={cloneIncludeSkills}
                onChange={(e) => setCloneIncludeSkills(e.target.checked)}
              />
              {t("agents.clone_include_skills", { defaultValue: "Copy skill assignments" })}
            </label>
            <label className="flex items-center gap-2 text-[11px] text-text-dim">
              <input
                type="checkbox"
                checked={cloneIncludeTools}
                onChange={(e) => setCloneIncludeTools(e.target.checked)}
              />
              {t("agents.clone_include_tools", { defaultValue: "Copy tool assignments" })}
            </label>
          </div>
          <div className="flex justify-end gap-2 pt-2">
            <Button type="button" variant="secondary" onClick={() => setCloneDialog(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              type="submit"
              variant="primary"
              disabled={!cloneNameDraft.trim() || cloneMutation.isPending}
            >
              {cloneMutation.isPending
                ? t("common.saving", { defaultValue: "Saving..." })
                : t("agents.clone")}
            </Button>
          </div>
        </form>
      </Modal>
    </div>
  );
}
