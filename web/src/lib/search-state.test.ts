import { describe, expect, it } from 'vitest';
import {
	DEFAULT_SEARCH_STATE,
	hasFilters,
	needsSeed,
	parseSearchState,
	searchHref,
	searchStateToQuery,
	sortOnly,
	toWorksQuery,
	updateSearchState
} from './search-state';

const parse = (query: string) => parseSearchState(new URLSearchParams(query));

describe('parseSearchState', () => {
	it('reads every parameter', () => {
		expect(parse('q=%E8%80%B3&tag=7&circle=3&va=va-1&root=local&sort=rating&order=asc&page=3')).toEqual({
			q: '耳',
			tag: 7,
			circle: 3,
			va: 'va-1',
			root: 'local',
			sort: 'rating',
			order: 'asc',
			seed: null,
			page: 3
		});
	});

	it('falls back to defaults for values the server would reject', () => {
		expect(parse('tag=abc&circle=-1&sort=popular&order=sideways&seed=-5&page=0&va=&root=')).toEqual(
			DEFAULT_SEARCH_STATE
		);
		expect(parse('tag=1.5&page=2x').tag).toBeNull();
	});

	it('keeps a seed of zero', () => {
		expect(parse('sort=random&seed=0').seed).toBe(0);
	});
});

describe('searchStateToQuery', () => {
	it('leaves defaults out so the bare page has a bare URL', () => {
		expect(searchStateToQuery(DEFAULT_SEARCH_STATE)).toBe('');
		expect(searchHref('/search', DEFAULT_SEARCH_STATE)).toBe('/search');
	});

	it('round-trips through the URL', () => {
		const state = { ...DEFAULT_SEARCH_STATE, q: 'RJ01 耳语', tag: 9, va: 'va 2', sort: 'release' as const, order: 'asc' as const, page: 2 };
		expect(parse(searchStateToQuery(state))).toEqual(state);
	});

	it('writes the seed only for random and the order only for the others', () => {
		const random = { ...DEFAULT_SEARCH_STATE, sort: 'random' as const, order: 'asc' as const, seed: 42 };
		expect(searchStateToQuery(random)).toBe('sort=random&seed=42');
		expect(searchStateToQuery({ ...random, sort: 'rating' })).toBe('sort=rating&order=asc');
	});
});

describe('updateSearchState', () => {
	const onPage3 = { ...DEFAULT_SEARCH_STATE, q: 'a', page: 3 };

	it('goes back to page 1 when the results change', () => {
		expect(updateSearchState(onPage3, { tag: 7 }).page).toBe(1);
		expect(updateSearchState(onPage3, { sort: 'rating' }).page).toBe(1);
	});

	it('keeps the rest when only the page changes', () => {
		expect(updateSearchState(onPage3, { page: 4 })).toEqual({ ...onPage3, page: 4 });
	});

	it('picks a seed when switching to random and drops it when leaving', () => {
		const random = updateSearchState(onPage3, { sort: 'random' }, () => 0.5);
		expect(random.seed).toBe(2 ** 30);
		expect(needsSeed(random)).toBe(false);
		// Paging a random order keeps its seed, so pages do not overlap.
		expect(updateSearchState(random, { page: 2 }, () => 0.9).seed).toBe(2 ** 30);
		expect(updateSearchState(random, { sort: 'added' }).seed).toBeNull();
	});

	it('accepts an explicit new seed for shuffling again', () => {
		const random = { ...DEFAULT_SEARCH_STATE, sort: 'random' as const, seed: 1, page: 2 };
		expect(updateSearchState(random, { seed: 99 })).toMatchObject({ seed: 99, page: 1 });
	});

	it('fills in a seed for a random URL that has none', () => {
		const bare = parse('sort=random&q=x');
		expect(needsSeed(bare)).toBe(true);
		expect(updateSearchState(bare, {}, () => 0).seed).toBe(0);
	});
});

describe('toWorksQuery', () => {
	it('sends only what differs from the server defaults', () => {
		expect(toWorksQuery(DEFAULT_SEARCH_STATE, 40)).toEqual({ page: 1, page_size: 40 });
	});

	it('trims the text and maps every filter', () => {
		const state = parse('q=%20%20RJ01%20&tag=7&circle=3&va=v&root=r&sort=release&order=asc&page=2');
		expect(toWorksQuery(state, 40)).toEqual({
			q: 'RJ01',
			tag: 7,
			circle: 3,
			va: 'v',
			root: 'r',
			sort: 'release',
			order: 'asc',
			page: 2,
			page_size: 40
		});
	});

	it('sends the seed instead of the order for random', () => {
		expect(toWorksQuery(parse('sort=random&seed=5&order=asc'), 40)).toEqual({
			sort: 'random',
			seed: 5,
			page: 1,
			page_size: 40
		});
	});
});

describe('hasFilters and sortOnly', () => {
	it('treats blank text as no search', () => {
		expect(hasFilters(parse('q=%20%20'))).toBe(false);
		expect(hasFilters(parse('root=local'))).toBe(true);
	});

	it('keeps only the sort and page for the library', () => {
		expect(sortOnly(parse('q=x&tag=7&sort=rating&page=2'))).toEqual({
			...DEFAULT_SEARCH_STATE,
			sort: 'rating',
			page: 2
		});
	});
});
