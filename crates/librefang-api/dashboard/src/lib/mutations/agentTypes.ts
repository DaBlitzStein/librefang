import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  createAgentType,
  updateAgentType,
  deleteAgentType,
  spawnEphemeral,
  type AgentTypeInput,
  type EphemeralSpawnRequest,
} from "../http/client";
import { agentTypeKeys } from "../queries/keys";

export function useCreateAgentType() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: AgentTypeInput) => createAgentType(body),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: agentTypeKeys.all });
    },
  });
}

/**
 * Save an edit to an existing agent type.
 *
 * The body preserves every manifest field the operator did not touch (#7740):
 * the editor round-trips the type's full `manifest_toml`, so an operator's
 * `[[triggers]]`, `tool_allowlist`, `[compaction]` and the rest survive a save
 * rather than being flattened to the handful of fields a form can show.
 */
export function useUpdateAgentType() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ name, body }: { name: string; body: AgentTypeInput }) =>
      updateAgentType(name, body),
    onSuccess: (_data, { name }) => {
      qc.invalidateQueries({ queryKey: agentTypeKeys.detail(name) });
      qc.invalidateQueries({ queryKey: agentTypeKeys.lists() });
    },
  });
}

export function useDeleteAgentType() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => deleteAgentType(name),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: agentTypeKeys.all });
    },
  });
}

// A one-shot ephemeral run creates and tears down a transient worker with no
// registry entry, so nothing in the agent-type list changes — no invalidation
// needed. The result is returned to the caller for display.
export function useSpawnEphemeral() {
  return useMutation({
    mutationFn: (body: EphemeralSpawnRequest) => spawnEphemeral(body),
  });
}
