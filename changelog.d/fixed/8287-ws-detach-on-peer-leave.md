A chat you switch away from mid-turn no longer leaves the next visit waiting.
The WebSocket loop awaited the whole agent turn, so while one ran the daemon read nothing from the socket: a liveness probe went unanswered, and a peer that closed the connection stayed invisible until the turn finished on its own.
The loop now polls the socket alongside the turn — pings are answered mid-turn, any other frame is deferred and replayed in order, and a peer going away ends the wait immediately.
Leaving detaches rather than cancels: the agent loop is a task the kernel already spawned, so the turn finishes and persists to the session and the answer is there when you come back (#8287) (@DaBlitzStein)
