The agent editor exposes the remaining sampling parameters as typed fields: context window, `top_p`, `frequency_penalty`, and `presence_penalty`.
`top_p` and the penalties are real `ModelConfig` fields now instead of untyped `extra_params` keys, so the manifest form, the TOML, and the request body all express one validated value.
`reasoning_effort` stays a per-model setting, not an agent manifest field, matching the endpoint-fact rule from #7770.
A template name on `POST /api/agents` now also resolves from the `agent-types/` store, preferring it on a collision with a live agent's manifest exactly as the template catalog does.
Out-of-range sampling values are clamped at save time instead of being written into the manifest TOML.
(#8112) (@DaBlitzStein)
