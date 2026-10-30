const CACHE_NAME = 'silicate-pwa';
const CACHE_EXPIRATION_TIME = 24 * 60 * 60 * 1000; // 24 hours

var filesToCache = [
  './',
  './index.html',
  './silicate.js',
  './silicate_bg.wasm',
];

/* Start the service worker and cache all of the app's content */
self.addEventListener('install', function (e) {
  self.skipWaiting();
  e.waitUntil(
    caches.open(CACHE_NAME).then(function (cache) {
      return cache.addAll(filesToCache);
    })
  );
});

/* Take over the page that registered us, so it can reload with isolation headers */
self.addEventListener('activate', (event) => {
  event.waitUntil(self.clients.claim());
});

/*
 * WebAssembly threads need SharedArrayBuffer, which browsers only expose on
 * cross-origin isolated pages. GitHub Pages can't set response headers, so
 * add them here instead.
 */
function withIsolationHeaders(response) {
  // Opaque responses can't be rewrapped.
  if (!response || response.status === 0) {
    return response;
  }
  const headers = new Headers(response.headers);
  headers.set('Cross-Origin-Opener-Policy', 'same-origin');
  headers.set('Cross-Origin-Embedder-Policy', 'require-corp');
  headers.set('Cross-Origin-Resource-Policy', 'same-origin');
  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
}

/* Serve */
self.addEventListener('fetch', (event) => {
  event.respondWith(
    caches.open(CACHE_NAME).then(async (cache) => {
      const cachedResponse = await cache.match(event.request);
      const now = Date.now();
      if (cachedResponse) {
        const cachedTime = cachedResponse.headers.get('sw-cache-time');
        if (now - cachedTime < CACHE_EXPIRATION_TIME) {
          return withIsolationHeaders(cachedResponse);
        }
      }
      const networkResponse = await fetch(event.request);
      // if response invalid, return cached response if available
      if (!networkResponse || networkResponse.status !== 200) {
        return withIsolationHeaders(cachedResponse || networkResponse);
      }

      // Clone the response and add the timestamp header before caching
      const responseWithTimestamp = networkResponse.clone();
      let headers = new Headers(responseWithTimestamp.headers);
      headers.append('sw-cache-time', now.toString());
      responseWithTimestamp.headers = headers;

      cache.put(event.request, responseWithTimestamp);
      return withIsolationHeaders(networkResponse);
    })
  );
});
