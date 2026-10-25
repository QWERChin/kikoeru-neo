import { describe, expect, it } from 'vitest';
import { RecentSearches } from './recent-searches.svelte';

describe('RecentSearches', () => {
	it('shows a remembered term at once, without storage', () => {
		const recent = new RecentSearches();
		expect(recent.list).toEqual([]);
		recent.remember('耳语');
		recent.remember('RJ01');
		expect(recent.list).toEqual(['RJ01', '耳语']);
		recent.clear();
		expect(recent.list).toEqual([]);
	});
});
