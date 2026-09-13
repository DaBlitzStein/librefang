// Bumping this name is what evicts a poisoned cache from a browser already carrying one: `activate` deletes every cache whose name differs.
const CACHE_NAME = "librefang-v2";
const MAX_CACHE_ENTRIES = 200;

async function trimCache(cache) {
  const keys = await cache.keys();
  const excess = keys.length - MAX_CACHE_ENTRIES;
  if (excess <= 0) return;
  await Promise.all(keys.slice(0, excess).map((request) => cache.delete(request)));
}

// The worker this replaces never called `skipWaiting()`, and the only thing that could activate a waiting replacement was a `SKIP_WAITING` message no dashboard code has ever sent.
// A new worker therefore sat waiting until every tab in scope closed, which is not something an operator who keeps the dashboard open ever does.
self.addEventListener("install", () => {
  self.skipWaiting();
});

self.addEventListener("activate", (e) => {
  e.waitUntil(
    caches.keys().then(async (names) => {
      await Promise.all(names.filter((n) => n !== CACHE_NAME).map((n) => caches.delete(n)));
      // Claim the open tabs so their next asset fetch goes through this worker rather than the predecessor that was serving the stale bundle.
      await self.clients.claim();
      const cache = await caches.open(CACHE_NAME);
      await trimCache(cache);
    }),
  );
});

self.addEventListener("fetch", (e) => {
  const url = new URL(e.request.url);

  // Only handle http(s) requests
  if (!url.protocol.startsWith("http")) return;

  // API requests: network only
  if (url.pathname.startsWith("/api/")) return;

  // Only cache GET requests (Cache API does not support POST)
  if (e.request.method !== "GET") return;

  // Navigations: network only, never cached.
  // The HTML shell names the hashed asset bundle of the build it came from, so a shell replayed from cache after a redeploy asks for chunks the server no longer has.
  // Serving it stale-while-revalidate still hands the stale copy to the navigation that triggered the revalidation, which is the one that matters.
  // Hashed assets stay cacheable because their URL changes with their content.
  if (e.request.mode === "navigate") return;

  // Static assets: stale-while-revalidate
  e.respondWith(
    caches.open(CACHE_NAME).then(async (cache) => {
      const cached = await cache.match(e.request);
      const fetched = fetch(e.request)
        .then(async (resp) => {
          if (resp.ok) {
            await cache.put(e.request, resp.clone());
            await trimCache(cache);
          }
          return resp;
        });
      if (cached) {
        e.waitUntil(fetched.catch(() => undefined));
        return cached;
      }
      return fetched.catch(() => Response.error());
    }),
  );
});
