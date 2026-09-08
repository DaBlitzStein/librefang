`update_manifest` no longer reprojects an agent's tags twice per manifest PATCH.
It routed a tags change through `registry::update_tags` and then, on the very next line, called `replace_manifest_and_retag` — which already reprojects `entry.tags` and the `tag_index` from `manifest.tags` as part of the same call (#7742).
Both calls fired `notify_changed()`, so every `AgentRegistry` watcher woke twice per PATCH that changed tags, and a failure in `replace_manifest_and_retag` after `update_tags` had already written the new tags left `entry.tags` ahead of `entry.manifest` until the next successful write.
The redundant call is gone; `replace_manifest_and_retag` alone was already doing the whole job. (#7835) (@DaBlitzStein)
