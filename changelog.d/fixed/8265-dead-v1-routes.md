The OpenAPI spec the daemon serves no longer advertises `/api/v1/versions`, a route it answers with a 404.
The spec handler copies every `/api/*` path under `/api/v1/*`, which is right for the routes nested at both prefixes and wrong for the few registered directly on the app.
Version discovery is one of those, and unversioned on purpose: it is the endpoint a client reaches before it knows which version to ask for.
The larger part of the fix is the dead-route audit, which could not have caught this for three separate reasons and has been repaired for all of them.
It read the static document rather than the spec as served, so none of the 345 `/api/v1/*` paths had ever been dispatched — it audited 353 paths against 698 served.
It recognised only axum's top-level fallback, `text/plain` with the body `Not Found`, and a nested router answers a bare 404 with no content type at all, which the audit read as a live route; it now asserts the invariant the codebase actually holds, that every real 404 is JSON.
And it tripped the request rate limiter after 82 dispatches, after which every reply was a 429 rather than a 404 — four fifths of the audit was inert and would have stayed green with four fifths of the routes deleted. (#8265) (@DaBlitzStein)
