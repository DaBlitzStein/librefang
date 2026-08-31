Agent types can be promoted to the public registry as a GitHub PR via `POST /api/templates/{name}/promote`, the dashboard Promote button, or the TUI `[p]` key.
The manifest is sanitized for publication before submission, reusing the same fork/push/PR machinery the skill propose flow uses.
(#8027) (@DaBlitzStein)
