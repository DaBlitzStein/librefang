# Recovering after a binary downgrade

This is the ops-facing answer to: *I ran a newer LibreFang binary, its first boot migrated the SQLite database forward, and now I want to go back to the older binary — what happens, and what do I do?*

Short version: the older binary refuses to boot by design, nothing is corrupt, and the way back is restoring a backup taken before the upgrade.

## What happens when an older binary meets a newer database

Every schema change ships in the PR that consumes it and takes the next free migration number at merge time.
At boot the memory substrate runs the migration ladder (`crates/librefang-memory/src/migration.rs`), and `SCHEMA_VERSION` at `migration.rs:8` is the highest step that binary knows.
When the database's `user_version` is higher than that, the guard at `migration.rs:16-28` refuses to run anything and the daemon does not start (`crates/librefang-memory/src/substrate.rs:217` propagates the error out of the substrate constructor):

```
Database schema version 58 is newer than this binary supports (54). Downgrade is not supported. Use the correct binary version or restore from backup.
```

The refusal is deliberate.
Before #3962 (April 2026), an older daemon on a newer database *silently rewrote `user_version` down* and kept booting.
That was issue #3656: the operator saw NULL and missing-column errors on the newer tables, and the next forward upgrade re-ran migrations against a schema they had partly applied.
The guard added by #3962 (together with per-step migration transactions and the daemon file lock) turns that silent corruption into a refusal to start with an instruction.

There is no downgrade migration ladder on purpose.
Migrations are additive — new tables, columns and indexes that the newer binary's code consumes — and the older binary has no DDL to remove them and no code that understands the schema without them.
Pre-declaring the next versions' schema before those PRs land was reviewed and rejected (#8068): a stub migration claims a version number, `run_step!` is gated on `current_version < version` (`migration.rs:95-104`), and the real migration behind that number then never runs on such a database — a silently diverged schema with no error anywhere.

## What a backup covers

The daemon ships a backup feature: `POST /api/backup`, or Admin → Runtime → Backups in the dashboard.
It walks the home directory and writes `librefang_backup_<timestamp>.zip` into `<home_dir>/backups/` (`crates/librefang-api/src/routes/backup.rs:243-249`).
The archive holds `config.toml`, agent workspaces, skills, workflows and the whole `data/` tree (`BACKUP_LAYOUT`, `backup.rs:68-80`) — the SQLite databases themselves included, since they live under `data/` (`backup.rs:265-269`): the memory database defaults to `data/librefang.db` (`crates/librefang-kernel/src/kernel/boot.rs:378-383`) and the A2A task store to `data/a2a_tasks.db` (`boot.rs:1811`).
Only the SQLite `-shm` shared-memory index sidecar is left out of the archive, on both backup and restore: it is the WAL index for the connections currently mapping the database, not state, and SQLite recreates it on demand (`backup.rs:185-192`, skipped at `backup.rs:314` and `backup.rs:949`).

A backup is a live file-level copy taken while the daemon runs.
It is not a transactionally consistent SQLite snapshot, so for the cleanest artifact create it while the daemon is quiet.

## The sanctioned rollback procedure

The recovery needs a backup taken before the upgrade — while the old binary still runs.
If you have one:

1. Do not force the older binary onto the migrated database; the guard will keep refusing, and that is correct.
2. Boot the **newer** binary once more — it accepts the database at its own version and gives you the API again.
3. Restore the pre-upgrade archive: `POST /api/restore` with `{"filename": "librefang_backup_<timestamp>.zip"}` (`backup.rs:1023-1041`), or the Backups tab's Restore control.
   Omitting `components` restores everything; `keep_config` (default `false`) means the archive's `config.toml` is applied too — set it `true` to keep the target's own instead.
   The restore overwrites home-directory state including the `data/` databases, which returns `user_version` to the pre-upgrade value.
4. Stop the daemon and start the **older** binary.
   The guard now passes, because the database is back at the version that binary shipped with.

The restore endpoint's own contract warns that a restore overwrites existing state files and that the daemon must be restarted afterwards (`backup.rs:1023-1041`) — step 4 is that restart, with the older binary in place.
Anything the newer binary wrote between the backup and the downgrade is gone; that is the price of the rollback.

If the daemon is down and cannot be brought up, the same result is available by hand: stop every daemon process, and unzip the archive over the home directory.
That is the same write the endpoint performs, minus the `-shm` filtering — delete any `-shm` file it leaves behind (the endpoint does, `backup.rs:949`; SQLite rebuilds it).

## If there is no backup

There is no in-product downgrade: the guard has no reverse ladder, and the error message already says the two supported exits are the correct binary version or a backup restore.
If neither exists, the options are:

- Stay on the newer binary — always safe, and usually the right call.
- Start fresh: stop the daemon, move `<home_dir>/data/` aside, and let the old binary build an empty database at its own version.
  Configuration, agent workspaces, skills and workflows survive (they are separate trees in the backup layout, `backup.rs:68-80`); memory, sessions, audit trail and every other `data/` artefact are lost.

## See also

- [`config-reload.md`](./config-reload.md) — which config changes take effect without a restart; `config.toml` is not migrated, so a downgrade does not touch it.
- [`audit-chain-recovery.md`](./audit-chain-recovery.md) — the comparable recovery procedure for a broken audit chain, which documents when *not* to repair.