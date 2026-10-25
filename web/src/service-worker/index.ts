import { version } from '$app/env';
import { assets, immutable } from '$app/manifest';
import { self as sw } from '$app/service-worker';
import { ACTIVE_KEY, CACHE_PREFIX, META_CACHE, cachesToDelete, shouldPrecache } from './caches';

// Precaches only the app's own files: the hashed build output, static/ and
// the app shell. API answers, covers and media are per user and change with
// every scan, so they always go to the network (book: frontend.md, PWA).
const CACHE = `${CACHE_PREFIX}${version}`;
/** The SPA fallback page. The server answers every page path with it. */
const SHELL = '/index.html';
const ASSETS = new Set([
	SHELL,
	...[...immutable, ...assets]
		.map((file) => `/${file.path.replace(/^\//, '')}`)
		.filter(shouldPrecache)
]);

sw.addEventListener('install', (event) => {
	// No skipWaiting here: the new version waits until the page accepts the
	// update prompt, so a release never swaps code under a playing track.
	// A failed precache must not leave a half-filled cache that the next
	// activation would mistake for a version some tab still runs.
	event.waitUntil(
		caches
			.open(CACHE)
			.then((cache) => cache.addAll([...ASSETS]))
			.catch(async (error: unknown) => {
				await caches.delete(CACHE);
				throw error;
			})
	);
});

sw.addEventListener('activate', (event) => {
	event.waitUntil(
		caches.keys().then(async (keys) => {
			// addAll stores every file or none, so a cache holding the shell
			// finished installing.
			const complete = new Set<string>();
			for (const key of keys) {
				if (await (await caches.open(key)).match(SHELL)) complete.add(key);
			}
			const meta = await caches.open(META_CACHE);
			const recorded = await meta.match(ACTIVE_KEY);
			const previous = recorded ? await recorded.text() : null;
			await Promise.all(
				cachesToDelete(keys, CACHE, previous, complete).map((k) => caches.delete(k))
			);
			// Recorded only once this version is the active one, so a version
			// that installs and then waits never counts as previously active.
			await meta.put(ACTIVE_KEY, new Response(CACHE));
		})
	);
});

sw.addEventListener('message', (event) => {
	if (event.data === 'skip-waiting') void sw.skipWaiting();
});

sw.addEventListener('fetch', (event) => {
	const { request } = event;
	if (request.method !== 'GET') return;
	const url = new URL(request.url);
	if (url.origin !== sw.location.origin) return;
	if (url.pathname === '/api' || url.pathname.startsWith('/api/')) return;

	if (request.mode === 'navigate') {
		event.respondWith(cachedOrFetch(SHELL, request));
		return;
	}
	if (ASSETS.has(url.pathname)) {
		event.respondWith(cachedOrFetch(url.pathname, request));
		return;
	}
	// A hashed chunk of the previous version, for a tab that declined the
	// update and still runs that code. The server may no longer have it.
	if (url.pathname.startsWith('/_app/immutable/')) {
		event.respondWith(caches.match(url.pathname).then((cached) => cached ?? fetch(request)));
	}
});

async function cachedOrFetch(key: string, request: Request): Promise<Response> {
	const cached = await caches.match(key, { cacheName: CACHE });
	return cached ?? fetch(request);
}
