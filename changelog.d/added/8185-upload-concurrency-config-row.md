Surfaced the new `max_concurrent_uploads` cap in the dashboard ConfigPage, next to the two size caps it works against.
The setting shipped as a config-file-only knob, which left an operator who hit a 429 with no way to see the limit that produced it, let alone raise it, without editing `config.toml` by hand and restarting.
It now renders as a `general` row and accepts writes through `POST /api/config/set`, matching `max_request_body_bytes` and `max_upload_size_bytes` — restart-required all three, which `config_reload.rs` already reports (#8185) (@DaBlitzStein)
