# Database migrations: the numbering convention

The SQLite substrate upgrades through a ladder of numbered steps in `crates/librefang-memory/src/migration.rs`.
Three PRs claimed overlapping numbers on one day, and the two failure modes that produced look nothing alike — one is a compile error, the other is silent and reaches only upgraded installations.
This page is the convention, and `migration::tests::the_migration_ladder_is_contiguous_and_named_for_its_versions` is the part of it the compiler can check.

## The rule

Adding a migration is four edits that move together:

1. A function named `migrate_v{N}`, where `{N}` is one past the current last step.
2. A `run_step!({N}, migrate_v{N});` line, appended after the current last one.
3. `const SCHEMA_VERSION: u32 = {N};` at the top of the file.
4. An audit row, recorded by `migrate_v{N}` itself under version `{N}` — not under `{N}-1`, and not left to the backfill.

Nothing may be skipped and nothing may be claimed twice.
A number is taken the moment a PR that uses it merges, so **whoever merges second renumbers**, and renumbering means moving all four edits, not just the `run_step!` line.

## Why the two failure modes need writing down

### A duplicate number is silent, and only on upgraded installs

`run_step!` dispatches on the pragma:

```rust
if current_version < $version {
    let tx = conn.unchecked_transaction()?;
    $migrate_fn(&tx)?;
    set_schema_version(&tx, $version)?;
    tx.commit()?;
}
```

On a database that already applied one v55, `current_version < 55` is false, so a *second* v55 body never runs and the table it creates is never created.
The feature then returns 500 on upgraded installations while working perfectly on fresh ones — and CI only ever sees the fresh path, because every test database starts at `user_version == 0` and runs every step.

The duplicate `fn` name announces itself as a compile error, which is why this looks handled.
It is not: resolving the collision by renumbering only the audit `INSERT`, or by bumping `SCHEMA_VERSION` without adding the `run_step!` line, produces the silent version with nothing red anywhere.

### A gap is caught, but the message points elsewhere

Jumping from `run_step!(54, …)` to `run_step!(57, …)` leaves a fresh database at `user_version = 57` with audit rows for 50–54 and 57.
`test_every_migration_records_audit_row` does go red, but it reports:

> a fresh ladder must need no audit-row backfill; some migrate_vN is recording its audit row under a version other than its own

Every migration on such a branch records its own number correctly.
The message describes a different defect, so the reader looks in the wrong place.

## What the guard checks, and what it cannot

`the_migration_ladder_is_contiguous_and_named_for_its_versions` parses the `run_step!` lines out of the module's own source and asserts the ladder is strictly increasing, contiguous, ends exactly at `SCHEMA_VERSION`, and that every step calls the function named for its own version.
It reads the source rather than a migrated database on purpose: the duplicate is invisible at runtime on the only path CI takes.

It cannot check that `migrate_v{N}`'s body does what its name and audit description claim.
Two sibling tests cover the part it cannot: `test_every_migration_records_audit_row` pins that each applied version has an audit row, and `no_migration_relies_on_the_audit_backfill` pins that each migration wrote that row itself rather than being rescued by the #3538 repair path.

## When you renumber

Assert against `SCHEMA_VERSION`, never against the literal.

A test that ends `assert_eq!(get_schema_version(&conn).unwrap(), 60)` is asserting "and is carried the rest of the way", and the constant says that without going stale the next time the ladder moves.
Three such assertions went stale in one renumber; the same file already used the constant form two hundred lines earlier.
