// User groups (#7745) — read-only.
//
// There are no mutation hooks in this file on purpose. Membership is declared
// in `config.toml` under `[[user_groups]]` and resolved in memory by the
// kernel's `AuthManager`; nothing is stored, so there is no record a write
// endpoint could update. Changing a group is a config edit followed by
// `POST /api/config/reload`, which is classified hot — no daemon restart.

import { useQuery } from "@tanstack/react-query";
import { getUserGroup, listUserGroups, type UserGroupItem } from "../http/client";
import { userGroupKeys } from "./keys";

export type { UserGroupItem };

export function useUserGroups() {
  return useQuery({
    queryKey: userGroupKeys.list(),
    queryFn: listUserGroups,
  });
}

/**
 * One group by its stable `id`.
 *
 * Keyed on the id rather than the display name because the two are separate
 * fields precisely so a rename does not orphan references to the group.
 */
export function useUserGroup(id: string | undefined) {
  return useQuery({
    queryKey: userGroupKeys.detail(id ?? ""),
    queryFn: () => getUserGroup(id as string),
    enabled: Boolean(id),
  });
}
