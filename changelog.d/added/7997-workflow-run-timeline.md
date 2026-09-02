Add a workflow run timeline to the dashboard's Workflows page, surfacing each run's outcome once it completes.
The task board gains canvas helpers and richer run status so an operator can follow a workflow from trigger to result without leaving the dashboard.
The workflow JSON extractor's brace-matching fallback now retries from the next `{` when a balanced candidate lacks the `"steps"` key, so a template placeholder like `{{name}}` ahead of the real payload no longer drops it. (#7997) (@DaBlitzStein)
