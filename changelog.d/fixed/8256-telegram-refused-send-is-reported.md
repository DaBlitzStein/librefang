A Telegram reply the API refuses is now reported instead of vanishing.
Both halves of the send path discarded the verdict: the daemon's `send` frame is fire-and-forget with no response frame, so it can only report a failed write to the sidecar's stdin, and the adapter dropped the API response on the floor.
A `403 bot was blocked by the user` therefore produced no journal line, no stderr and no counter, while the daemon logged the outbound message and reported success.
That turns "the agent stopped answering" into an investigation of the entire pipeline, because every hop that can be observed looks healthy and the one that failed cannot be.
The adapter now logs the verdict — method, chat id, HTTP status, error code and a truncated description, never the token and never the message body — and the daemon already forwards sidecar stderr into its own log, so the trace appears with no new plumbing.
Every chunk of a split reply is checked rather than only the first, so a refusal partway through a long answer is no longer hidden behind a delivered opening chunk.
The negotiation failure that makes `sendRichMessage` fall back to the legacy HTML pipeline stays silent, as does the happy path. (#8256) (@DaBlitzStein)
