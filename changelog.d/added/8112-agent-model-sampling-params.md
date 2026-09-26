Sampling overrides now reach every dispatch path — persistent, ephemeral (`/btw`), streaming, and ephemeral worker spawns — instead of only the persistent-session one.
The agent editor shows which providers honour `top_p` / `frequency_penalty` / `presence_penalty`, and the OpenAI reasoning-model and Ollama wire paths carry them: reasoning models withhold the knobs rather than answering an `unsupported_parameter` 400, and Ollama reads them from its native `options` object instead of ignoring a top-level key.
`reasoning_effort` stays a per-model setting, not an agent manifest field, matching the endpoint-fact rule from #7770.
A template name on `POST /api/agents` now also resolves from the `agent-types/` store, preferring it on a collision with a live agent's manifest exactly as the template catalog does.
A template that exists but cannot be read is reported as a server-side failure rather than as a missing template, and that verdict no longer depends on the caller's language: the status is decided where the error is raised instead of by matching English substrings of an already-translated message.
A bulk `POST /api/agents/bulk` failure now carries the same machine-readable `code` a single spawn returns.
Out-of-range sampling values are refused rather than written into the manifest TOML, so an operator learns the number was rejected instead of finding a value they never chose.
(#8112) (@DaBlitzStein)
