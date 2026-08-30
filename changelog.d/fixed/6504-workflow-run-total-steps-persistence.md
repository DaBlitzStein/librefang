A workflow run recovered after a daemon restart now reports its real step count instead of "step X of 0" in the API and dashboard.
`workflow_runs.total_steps` was added to the debug-UI run model but was never persisted, so `row_to_workflow_run` hardcoded `total_steps: 0` on every reload.
Migration v49 adds the column (`try_column_exists`-guarded `ALTER TABLE`, idempotent on rerun) and `workflow_run_to_row` / `row_to_workflow_run` now read and write the real value (#6504) (@DaBlitzStein)
