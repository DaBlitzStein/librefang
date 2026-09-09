Correct the guidance the `channel_send` system-channel guard puts in the prompt itself, which asserted three things that were not true.
It told a cron run nobody was watching its response, even though a configured delivery target hands that response straight to a real person.
It pointed the agent at `notify_owner` as a fallback on those same background runs, even though nothing on the cron or autonomous path ever reads the notice it queues.
It told `webui` that generated media is shown to the user automatically, even though the browser only ever sees what the reply text embeds.
The background-run and web-interface branches are now matched on the literal `cron` / `autonomous` / `webui` channel names instead of the shared reserved-channel-name list, so a future addition to that list no longer inherits background-run guidance by accident (#8149) (@DaBlitzStein)
