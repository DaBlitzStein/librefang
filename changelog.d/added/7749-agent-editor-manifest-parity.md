The running-agent editor can now reach the whole manifest, not the curated subset the detail projection exposes.
`GET /api/agents/{id}/manifest` returns the agent's manifest as raw TOML, and `PATCH /api/agents/{id}` accepts a `manifest_toml` field that replaces it wholesale.
The dashboard grows a full-manifest editor seeded from that read, plus Channels and Description sections and a per-server MCP grant toggle for fields that had no editing surface at all.
Fields the dashboard does not model are round-tripped verbatim through the TOML rather than dropped, so editing an agent from the UI no longer silently discards manifest keys the form never learned about. (#7749) (@DaBlitzStein)
