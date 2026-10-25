import { addRecentSearch, readRecentSearches, writeRecentSearches } from '../recent-searches';

/**
 * Recent searches shared by the search box that records them and the
 * search page that lists them, so a new term shows without a remount.
 */
export class RecentSearches {
	// Without storage, as in tests or a private window, this starts empty.
	#list = $state<string[]>(readRecentSearches());

	get list(): string[] {
		return this.#list;
	}

	remember(q: string): void {
		const next = addRecentSearch(this.list, q);
		this.#list = next;
		writeRecentSearches(next);
	}

	clear(): void {
		this.#list = [];
		writeRecentSearches([]);
	}
}

export const recentSearches = new RecentSearches();
