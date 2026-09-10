The dashboard is reachable again at its own basepath root.
`/dashboard` and `/dashboard/` answered 404 while `/dashboard/agents` answered 200, so a bookmark to the dashboard root failed, and an installed PWA — whose `start_url` is `/dashboard/#/overview` — launched straight into that 404.
A stuck service worker could separately pin a browser to a build the server no longer had: nothing ever sent the `SKIP_WAITING` message the worker relied on to hand over, so a replacement waited for every tab to close while the cached shell it kept serving named hashed chunks a redeploy had already removed.
Navigations now bypass the worker cache and a replacement takes over on install, so an ordinary reload recovers a browser that no amount of redeploying could fix before.
(#8260) (@DaBlitzStein)
