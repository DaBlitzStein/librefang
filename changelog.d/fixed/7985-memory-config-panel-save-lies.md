The TUI's memory config panel (Memory screen, `c`) no longer reports a save as clean when it was not.
Accepting a typed extraction model used to overwrite the boot-resolved model actually running before any PATCH was sent, so the panel claimed extraction had already moved to it; the resolved value is now left alone until a save confirms it, with the typed model shown as its own pending indicator.
An emptied model draft used to be accepted and then silently dropped by the save, since the endpoint has no way to unset that key; it now stays in the editor instead.
Pressing save with nothing edited used to PATCH the whole file anyway, which strips every comment out of `config.toml` on a full round trip through the endpoint's read-modify-write; it is now a no-op.
A second edit made while an earlier save was still in flight used to be folded into that save's "Saved" outcome and lost; the panel now only clears the unsaved marker when nothing changed since the save it belongs to.
The save request also inspected only the HTTP status, but the endpoint always returns 200 and reports a failed live reload through the response body — that case now shows as a failure instead of a silent "Saved", and the request carries a timeout sized for the reload it triggers rather than a plain GET.
(#7985) (@DaBlitzStein)
