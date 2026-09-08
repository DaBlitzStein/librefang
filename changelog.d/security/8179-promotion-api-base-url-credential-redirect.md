`skills.promotion.api_base_url` no longer accepts a plain `http://` origin unless the host is loopback, and `POST /api/config/set` now refuses to write the field at all.
Every request the registry promotion flow makes attaches the repo-scoped GitHub token as an `Authorization: Bearer` header, so a writable, unencrypted, or attacker-redirected value handed that credential to whatever host the field named.
The value now joins `proxy.http_proxy`, `telemetry.otlp_endpoint` and `audit.anchor_path` as an edit-on-disk destination field instead of a dashboard-tunable one. (#8179) (@DaBlitzStein)
