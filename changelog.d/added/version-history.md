Agent manifest version history: every config change is now recorded in SQLite so operators can see what changed and when.
New endpoint `GET /api/agents/{id}/manifest-history` returns timestamped TOML snapshots.
Dashboard gains a "History" tab on the agent detail panel showing the version timeline with expandable diffs.
Skills already had version history via the evolution system; this closes the gap for agents.
(@DaBlitzStein)
