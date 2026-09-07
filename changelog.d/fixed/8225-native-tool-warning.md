An agent deliberately configured without one of the always-native tools no longer logs a `WARN` claiming the tool is "missing from definitions".
The definition was never missing — `select_native_tools` receives the set granted to one agent, not the global registry, so every always-native tool the agent was not granted produced a warning per turn.
Chasing those in a production log cost a real misdiagnosis, sending the investigation after the tool registry and the manifest loader when the answer was an agent type that simply does not declare the tool.
The two cases are now told apart: a name with no entry in `builtin_tool_definitions()` is still a `WARN`, because it means someone extended `ALWAYS_NATIVE_TOOLS` without a definition, while a name the agent merely was not granted drops to `debug!` and says what is actually true.
(#PR) (@DaBlitzStein)
