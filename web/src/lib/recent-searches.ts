const STORAGE_KEY = 'kikoeru-neo.recent-searches';
export const MAX_RECENT_SEARCHES = 8;

/** Puts `q` first, drops an older copy of it and keeps the list short. */
export function addRecentSearch(list: readonly string[], q: string): string[] {
	const term = q.trim();
	if (term === '') return [...list];
	return [term, ...list.filter((item) => item !== term)].slice(0, MAX_RECENT_SEARCHES);
}

// Recent searches are a per-browser convenience. Storage can throw in
// private windows, and then the list simply starts empty every time.
export function readRecentSearches(): string[] {
	try {
		const parsed: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '[]');
		return Array.isArray(parsed)
			? parsed.filter((x): x is string => typeof x === 'string').slice(0, MAX_RECENT_SEARCHES)
			: [];
	} catch {
		return [];
	}
}

export function writeRecentSearches(list: readonly string[]): void {
	try {
		if (list.length === 0) localStorage.removeItem(STORAGE_KEY);
		else localStorage.setItem(STORAGE_KEY, JSON.stringify(list));
	} catch {
		// See readRecentSearches.
	}
}
