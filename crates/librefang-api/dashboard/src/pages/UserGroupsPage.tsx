// User groups page (#7745).
//
// Read-only, and that is a design decision rather than an unfinished screen.
// Groups are declared in `config.toml` under `[[user_groups]]` and their
// membership is resolved in memory by the kernel; nothing is persisted, so
// there is no record an edit button could write to. The page says so in a note
// instead of leaving an operator hunting for a missing "New group" control.
//
// All API access lives in `lib/queries/userGroups.ts`. This file only renders.

import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Users2, Search, UserRound, Info } from "lucide-react";

import type { UserGroupItem } from "../lib/http/client";
import { useUserGroups } from "../lib/queries/userGroups";
import { PageHeader } from "../components/ui/PageHeader";
import { Card } from "../components/ui/Card";
import { Badge } from "../components/ui/Badge";
import { Input } from "../components/ui/Input";
import { EmptyState } from "../components/ui/EmptyState";
import { CardSkeleton } from "../components/ui/Skeleton";

export function UserGroupsPage() {
  const { t } = useTranslation();
  const query = useUserGroups();
  const [search, setSearch] = useState("");

  // Matches id, display name, description and member names: an operator asking
  // "who is in support" and one asking "what is paco in" both start by typing
  // the thing they already know.
  const groups = useMemo(() => {
    const all: UserGroupItem[] = query.data ?? [];
    const needle = search.trim().toLowerCase();
    if (!needle) return all;
    return all.filter(
      (g) =>
        g.id.toLowerCase().includes(needle) ||
        g.name.toLowerCase().includes(needle) ||
        g.description.toLowerCase().includes(needle) ||
        g.members.some((m) => m.toLowerCase().includes(needle)),
    );
  }, [query.data, search]);

  const searchLabel = t("userGroups.searchPlaceholder", {
    defaultValue: "Search groups or members…",
  });

  return (
    <div className="space-y-6">
      <PageHeader
        icon={<Users2 className="h-5 w-5" />}
        title={t("userGroups.title", { defaultValue: "User Groups" })}
        subtitle={t("userGroups.subtitle", {
          defaultValue:
            "Named sets of users that can own things. Groups are flat — a group never contains another group.",
        })}
        isFetching={query.isFetching}
        onRefresh={() => void query.refetch()}
      />

      <Card className="flex items-start gap-3 text-sm">
        <Info
          className="mt-0.5 h-4 w-4 shrink-0 text-text-dim"
          aria-hidden="true"
        />
        <p className="text-text-dim">
          {t("userGroups.readOnlyNote", {
            defaultValue:
              "Groups are declared in config.toml under [[user_groups]], and membership is resolved in memory rather than stored. Edit the file and reload the config to change them — no restart needed.",
          })}
        </p>
      </Card>

      <Input
        leftIcon={<Search className="h-4 w-4" />}
        value={search}
        onChange={(e) => setSearch(e.target.value)}
        placeholder={searchLabel}
        aria-label={searchLabel}
      />

      {query.isPending ? (
        <CardSkeleton />
      ) : query.isError ? (
        <EmptyState
          icon={<Users2 className="h-7 w-7" />}
          title={t("userGroups.loadFailed", {
            defaultValue: "Could not load user groups",
          })}
        />
      ) : groups.length === 0 ? (
        <EmptyState
          icon={<Users2 className="h-7 w-7" />}
          title={
            search
              ? t("userGroups.noMatches", { defaultValue: "No matching groups" })
              : t("userGroups.empty", {
                  defaultValue: "No user groups declared",
                })
          }
          description={
            search
              ? undefined
              : t("userGroups.emptyHint", {
                  defaultValue:
                    "Add a [[user_groups]] block to config.toml to declare one.",
                })
          }
        />
      ) : (
        <div className="space-y-3">
          {groups.map((group) => (
            <UserGroupCard key={group.id} group={group} />
          ))}
        </div>
      )}
    </div>
  );
}

function UserGroupCard({ group }: { group: UserGroupItem }) {
  const { t } = useTranslation();

  return (
    <Card>
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="truncate font-bold tracking-tight">{group.name}</h3>
            {/* The id sits next to the name because it is what ownership
                records point at: an operator reading an owner needs to match it
                back to a group here, and the name may since have changed. */}
            <Badge variant="info">{group.id}</Badge>
          </div>
          {group.description ? (
            <p className="mt-1 text-sm text-text-dim">{group.description}</p>
          ) : null}
        </div>
        <Badge>
          {t("userGroups.memberCount", {
            count: group.member_count,
            defaultValue_one: "{{count}} member",
            defaultValue_other: "{{count}} members",
          })}
        </Badge>
      </div>

      {group.members.length > 0 ? (
        <ul className="mt-3 flex flex-wrap gap-2">
          {group.members.map((member) => (
            <li
              key={member}
              className="flex items-center gap-1.5 rounded-lg border border-border-subtle px-2 py-1 text-sm"
            >
              <UserRound
                className="h-3.5 w-3.5 text-text-dim"
                aria-hidden="true"
              />
              <span className="truncate">{member}</span>
            </li>
          ))}
        </ul>
      ) : (
        <p className="mt-3 text-sm text-text-dim">
          {t("userGroups.noMembers", { defaultValue: "No members declared" })}
        </p>
      )}
    </Card>
  );
}
