A sidecar child no longer receives the per-instance secrets belonging to its sibling instances.
The daemon logged that it was withholding another instance's `<PREFIX>__KEY` and then handed it over anyway: the merge declined to re-emit the key, but the daemon had already loaded the whole of `secrets.env` into its own process environment at boot and never calls `env_clear`, so the child inherited it.
Not setting a key isolates it only when nothing else supplies it, and here the parent did.
On a host running three Telegram instances that meant every adapter could read the other two bots' tokens while the boot log said otherwise.
The withheld keys are now unset on the child explicitly, before the merged pairs are applied so an operator's explicit `[sidecar_channels.env]` override still wins.
Only keys carrying the `__` namespace delimiter are removed, so an instance with no per-instance secrets keeps inheriting the bare global key it needs.
The regression test that was meant to guard this cleared the parent value before asserting, which deleted the very condition that produces the bug — it passed against the broken code, and the replacement asserts the child's effective environment instead. (#8254) (@DaBlitzStein)
