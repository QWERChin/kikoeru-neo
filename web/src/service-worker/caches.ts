export const CACHE_PREFIX = 'kikoeru-app-';
/** Holds which version was active last, outside the versioned caches. */
export const META_CACHE = 'kikoeru-meta';
export const ACTIVE_KEY = '/__kikoeru-active-version';

/**
 * Caches an activating worker may delete. The version that was active before
 * this one is kept, because a tab that declined the update still runs that
 * version's code and may still import its hashed chunks. A version that
 * installed but never activated (replaced while waiting) runs in no tab, so
 * it goes like every older one.
 *
 * `previous` is the cache that was active before, when it is known. Without
 * it (the first activation that records it), the newest previous cache that
 * finished installing is kept instead. `keys` is in creation order, as
 * `caches.keys()` returns it, and `complete` holds the keys whose precache
 * finished.
 */
export function cachesToDelete(
	keys: readonly string[],
	current: string,
	previous: string | null,
	complete: ReadonlySet<string>
): string[] {
	const old = keys.filter((key) => key.startsWith(CACHE_PREFIX) && key !== current);
	const keep =
		previous !== null && old.includes(previous)
			? previous
			: old.filter((key) => complete.has(key)).at(-1);
	return old.filter((key) => key !== keep);
}

/**
 * Whether a build file is fetched when a version installs. JASSUB's two
 * WebAssembly builds (about 2 MB each) and the fallback fonts (1.7 MB) are
 * needed only for videos with ass subtitles, so they load on first use like
 * any other request instead of on every update for every listener. The
 * interface uses system fonts, so every woff2 in the build is one of
 * JASSUB's fallback fonts.
 */
export function shouldPrecache(path: string): boolean {
	return !path.endsWith('.wasm') && !path.endsWith('.woff2');
}
