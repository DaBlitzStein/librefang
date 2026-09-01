Template (agent-type) version history: every create, edit, and restore is snapshotted in SQLite so operators can see how a template changed over time and one-click restore a prior configuration from the dashboard.
`GET /api/templates/{name}/history` lists snapshots; `POST /api/templates/{name}/history/{id}/restore` writes the stored TOML back to disk.
Skills already tracked version history via `.evolution.json` and are unchanged. (#8047) (@DaBlitzStein)
