The TUI can now show an agent's configuration history, which until now existed only in the HTTP API and the dashboard.
Press `h` on an agent's detail pane to list every recorded manifest snapshot newest-first, with the full TOML of the selected one beside it, so an operator working in a terminal can answer "what changed on this agent, and when" without opening a browser.
The pane distinguishes the three answers that would otherwise all look like an empty box: a fetch still in flight, an agent whose manifest was never persisted, and a request the daemon rejected — the last one now shows the reason the daemon gave instead of being dropped, which is also why a failed fetch on the agents tab no longer disappears silently.
Timestamps are read as the UTC the store writes and shown in local time, matching what the dashboard already displays.
(#PR) (@DaBlitzStein)
