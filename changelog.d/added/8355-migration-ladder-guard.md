The migration ladder now fails the build when a step is duplicated, skipped, renamed away from its own version, or left behind by `SCHEMA_VERSION`.
A duplicate number used to be invisible: `run_step!` fires on `current_version < N`, so a second step claiming a taken `N` never runs on an installation already past it, while a fresh database — which is the only kind CI ever creates — runs every step and shows nothing wrong.
The feature would then return 500 on upgraded installations and work perfectly on new ones.
The convention the guard enforces is written down in `docs/development/database-migrations.md`, along with why the existing gap check reports the symptom under a message about audit rows.
(#8355) (@DaBlitzStein)
