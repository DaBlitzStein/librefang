import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { join } from "node:path";
import { describe, expect, it, vi } from "vitest";

type WorkerHandler = (event: Record<string, unknown>) => void;

const SW_SOURCE = readFileSync(join(__dirname, "../../public/sw.js"), "utf8");
// Read back rather than restate, so bumping the name in `sw.js` to evict a
// poisoned cache does not silently invert the first-install test below.
const CACHE_NAME = SW_SOURCE.match(/CACHE_NAME = "([^"]+)"/)?.[1] ?? "";

class MemoryCache {
  entries = new Map<unknown, Response>();

  async match(request: unknown): Promise<Response | undefined> {
    return this.entries.get(request);
  }

  async put(request: unknown, response: Response): Promise<void> {
    this.entries.set(request, response);
  }

  async keys(): Promise<unknown[]> {
    return [...this.entries.keys()];
  }

  async delete(request: unknown): Promise<boolean> {
    return this.entries.delete(request);
  }
}

function loadWorker(
  fetchMock: typeof fetch,
  cache = new MemoryCache(),
  cacheNames = ["librefang-v0", "librefang-v1"],
) {
  const handlers = new Map<string, WorkerHandler>();
  const skipWaiting = vi.fn();
  const navigate = vi.fn(async () => undefined);
  const claim = vi.fn(async () => undefined);
  const windowClient = { url: "https://example.test/dashboard/agents", navigate };
  const worker = {
    addEventListener: (type: string, handler: WorkerHandler) => handlers.set(type, handler),
    skipWaiting,
    clients: { claim, matchAll: vi.fn(async () => [windowClient]) },
  };
  const caches = {
    open: vi.fn(async () => cache),
    keys: vi.fn(async () => cacheNames),
    delete: vi.fn(async () => true),
  };
  runInNewContext(SW_SOURCE, {
    self: worker,
    caches,
    fetch: fetchMock,
    Response,
    URL,
    console,
  });
  return { cache, caches, handlers, skipWaiting, claim, navigate };
}

describe("dashboard service worker", () => {
  it("returns an error Response when both cache and network miss", async () => {
    const fetchMock = vi.fn(async () => { throw new Error("offline"); }) as unknown as typeof fetch;
    const { handlers } = loadWorker(fetchMock);
    let responsePromise: Promise<Response> | undefined;
    handlers.get("fetch")?.({
      request: { url: "https://example.test/dashboard/missing.js", method: "GET" },
      respondWith: (promise: Promise<Response>) => { responsePromise = promise; },
      waitUntil: vi.fn(),
    });
    const response = await responsePromise;
    expect(response).toBeInstanceOf(Response);
    expect(response?.type).toBe("error");
  });

  it("keeps a cached response while refreshing it in the event lifetime", async () => {
    const request = { url: "https://example.test/dashboard/app.js", method: "GET" };
    const cached = new Response("cached");
    const fresh = new Response("fresh");
    const cache = new MemoryCache();
    cache.entries.set(request, cached);
    const { handlers } = loadWorker(vi.fn(async () => fresh) as unknown as typeof fetch, cache);
    let responsePromise: Promise<Response> | undefined;
    let refreshPromise: Promise<unknown> | undefined;
    handlers.get("fetch")?.({
      request,
      respondWith: (promise: Promise<Response>) => { responsePromise = promise; },
      waitUntil: (promise: Promise<unknown>) => { refreshPromise = promise; },
    });
    expect(await responsePromise).toBe(cached);
    await refreshPromise;
    expect(await cache.entries.get(request)?.text()).toBe("fresh");
  });

  it("bounds the runtime cache after a successful network response", async () => {
    const cache = new MemoryCache();
    for (let i = 0; i < 205; i++) cache.entries.set(`old-${i}`, new Response(String(i)));
    const request = { url: "https://example.test/dashboard/new.js", method: "GET" };
    const { handlers } = loadWorker(vi.fn(async () => new Response("new")) as unknown as typeof fetch, cache);
    let responsePromise: Promise<Response> | undefined;
    handlers.get("fetch")?.({
      request,
      respondWith: (promise: Promise<Response>) => { responsePromise = promise; },
      waitUntil: vi.fn(),
    });
    await responsePromise;
    expect(cache.entries.size).toBe(200);
    expect(cache.entries.has(request)).toBe(true);
  });

  it("never serves a navigation from cache", async () => {
    // The shell names the hashed bundle of the build it came from, so replaying
    // it after a redeploy points the page at chunks the server no longer has.
    const fetchMock = vi.fn(async () => new Response("fresh")) as unknown as typeof fetch;
    const { handlers } = loadWorker(fetchMock);
    const respondWith = vi.fn();
    handlers.get("fetch")?.({
      request: { url: "https://example.test/dashboard/agents", method: "GET", mode: "navigate" },
      respondWith,
      waitUntil: vi.fn(),
    });
    expect(respondWith).not.toHaveBeenCalled();
  });

  it("takes over open tabs on install instead of waiting for them to close", async () => {
    // A waiting worker can never replace a predecessor that is serving a broken
    // shell, because that shell is the page the user keeps reloading.
    const fetchMock = vi.fn(async () => new Response("x")) as unknown as typeof fetch;
    const { handlers, skipWaiting } = loadWorker(fetchMock);
    handlers.get("install")?.({ waitUntil: vi.fn() });
    expect(skipWaiting).toHaveBeenCalledOnce();
  });

  it("drops earlier cache generations and reloads the tabs they poisoned", async () => {
    const fetchMock = vi.fn(async () => new Response("x")) as unknown as typeof fetch;
    const { handlers, caches, claim, navigate } = loadWorker(fetchMock);
    let activatePromise: Promise<unknown> | undefined;
    handlers.get("activate")?.({
      waitUntil: (promise: Promise<unknown>) => { activatePromise = promise; },
    });
    await activatePromise;
    expect(caches.delete).toHaveBeenCalledWith("librefang-v0");
    expect(caches.delete).toHaveBeenCalledWith("librefang-v1");
    expect(claim).toHaveBeenCalledOnce();
    expect(navigate).toHaveBeenCalledWith("https://example.test/dashboard/agents");
  });

  it("leaves the page alone on a first-ever install", async () => {
    // Nothing stale to recover from, so reloading the tab the user just opened
    // would be gratuitous.
    const fetchMock = vi.fn(async () => new Response("x")) as unknown as typeof fetch;
    const { handlers, navigate } = loadWorker(fetchMock, new MemoryCache(), [CACHE_NAME]);
    let activatePromise: Promise<unknown> | undefined;
    handlers.get("activate")?.({
      waitUntil: (promise: Promise<unknown>) => { activatePromise = promise; },
    });
    await activatePromise;
    expect(navigate).not.toHaveBeenCalled();
  });
});
