// Bumping this name is what evicts a poisoned cache from a browser that is
// already carrying one: `activate` deletes every cache whose name differs.
const CACHE_NAME = "librefang-v2";
const MAX_CACHE_ENTRIES = 200;

async function trimCache(cache) {
  const keys = await cache.keys();
  const excess = keys.length - MAX_CACHE_ENTRIES;
  if (excess <= 0) return;
  await Promise.all(keys.slice(0, excess).map((request) => cache.delete(request)));
}

// Take over immediately rather than waiting for every tab in scope to close.
// A worker that waits can never replace a predecessor that is serving a broken
// shell, because the broken shell is the page the user keeps reloading.
self.addEventListener("install", () => {
  self.skipWaiting();
});

self.addEventListener("activate", (e) => {
  e.waitUntil(
    caches.keys().then(async (names) => {
      const stale = names.filter((n) => n !== CACHE_NAME);
      await Promise.all(stale.map((n) => caches.delete(n)));
      await self.clients.claim();
      const cache = await caches.open(CACHE_NAME);
      await trimCache(cache);
      // Only when a previous cache generation was actually discarded: reload
      // the open tabs, so a browser still displaying the stale shell recovers
      // without the user reloading twice.
      // A first-ever install finds no other cache and leaves the page alone.
      if (stale.length === 0) return;
      const windows = await self.clients.matchAll({ type: "window" });
      await Promise.all(
        windows.map((client) => client.navigate(client.url).catch(() => undefined)),
      );
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
  // The HTML shell names the hashed asset bundle of the build it came from, so
  // a shell replayed from cache after a redeploy asks for chunks the server no
  // longer has and renders a blank page that reloading cannot clear.
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
