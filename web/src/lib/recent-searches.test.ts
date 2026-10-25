import { describe, expect, it } from 'vitest';
import { MAX_RECENT_SEARCHES, addRecentSearch } from './recent-searches';

describe('addRecentSearch', () => {
	it('puts the newest first without duplicates', () => {
		expect(addRecentSearch(['a', 'b', 'c'], ' b ')).toEqual(['b', 'a', 'c']);
	});

	it('ignores blank text and caps the list', () => {
		expect(addRecentSearch(['a'], '   ')).toEqual(['a']);
		const full = Array.from({ length: MAX_RECENT_SEARCHES }, (_, i) => `q${i}`);
		const next = addRecentSearch(full, 'new');
		expect(next).toHaveLength(MAX_RECENT_SEARCHES);
		expect(next[0]).toBe('new');
		expect(next).not.toContain(`q${MAX_RECENT_SEARCHES - 1}`);
	});
});
