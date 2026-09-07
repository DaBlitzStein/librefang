`ToolExecConfig` gains `default_timeout_secs`, so the local tool-execution backend takes its default per-command timeout from configuration instead of a hardcoded 30 seconds.
`LocalBackend` documented `kernel config / agent manifest` as its source of truth while `ToolExecConfig` carried no timeout field at all, which left `build_backend` passing the constant on every path.
Leaving the new key unset inherits the global `tool_timeout_secs`, so an operator who moves that one knob does not silently leave the backend behind on a value they never chose.
The trait route is still opt-in and the tool runner continues to call the sandbox helpers directly, so this closes a gap in the backend's own configuration surface rather than changing what a running agent's commands do today.
(#8175) (@DaBlitzStein)
