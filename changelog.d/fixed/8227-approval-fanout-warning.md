An approval that was delivered no longer logs `Approval dropped` for every adapter that did not carry it.
The warning sat inside the broadcast loop, so it described one adapter's turn while being phrased as a verdict on the approval; a host running one sidecar per agent — a supported configuration — logged N-1 false alarms per approval and sent at least one investigation chasing routing config that was correct all along.
The per-adapter line is now a `debug!` that says what it means, and the warning is evaluated once after the fan-out, so a genuinely undeliverable approval still surfaces loudly with its request id and requesting agent.
(#PR) (@DaBlitzStein)
