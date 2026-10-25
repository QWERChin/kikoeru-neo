import { WORK_SORTS, type SortOrder, type WorkSort, type WorksQuery } from './api/query';

/**
 * Everything a works grid shows, kept in the URL query so refresh, back and
 * a shared link all land on the same results. Values the server would
 * reject are dropped while parsing, so a hand-edited URL still loads.
 */
export interface SearchState {
	q: string;
	tag: number | null;
	circle: number | null;
	va: string | null;
	root: string | null;
	sort: WorkSort;
	order: SortOrder;
	/** Only meaningful for `random`, where it keeps the order stable across pages. */
	seed: number | null;
	page: number;
}

/** Long enough to skip the keys of one word, short enough to feel live. */
export const SEARCH_DEBOUNCE_MS = 300;

export const DEFAULT_SEARCH_STATE: SearchState = {
	q: '',
	tag: null,
	circle: null,
	va: null,
	root: null,
	sort: 'added',
	order: 'desc',
	seed: null,
	page: 1
};

// Seeds stay below 2^31 so they read as short numbers in a shared URL.
const SEED_LIMIT = 2 ** 31;

function positiveInt(value: string | null): number | null {
	if (value === null || !/^\d+$/.test(value)) return null;
	const n = Number(value);
	return Number.isSafeInteger(n) && n > 0 ? n : null;
}

function nonNegativeInt(value: string | null): number | null {
	if (value === null || !/^\d+$/.test(value)) return null;
	const n = Number(value);
	return Number.isSafeInteger(n) ? n : null;
}

function nonEmpty(value: string | null): string | null {
	return value === null || value === '' ? null : value;
}

/** Accepts `page.url.searchParams`, which SvelteKit types as read-only. */
export function parseSearchState(params: Pick<URLSearchParams, 'get'>): SearchState {
	const sort = params.get('sort');
	return {
		q: params.get('q') ?? '',
		tag: positiveInt(params.get('tag')),
		circle: positiveInt(params.get('circle')),
		va: nonEmpty(params.get('va')),
		root: nonEmpty(params.get('root')),
		sort: (WORK_SORTS as readonly string[]).includes(sort ?? '') ? (sort as WorkSort) : 'added',
		order: params.get('order') === 'asc' ? 'asc' : 'desc',
		seed: nonNegativeInt(params.get('seed')),
		page: positiveInt(params.get('page')) ?? 1
	};
}

/**
 * The query string for a state, without the leading `?`. Defaults are left
 * out and keys keep a fixed order, so equal states give equal URLs.
 */
export function searchStateToQuery(state: SearchState): string {
	const params = new URLSearchParams();
	if (state.q !== '') params.set('q', state.q);
	if (state.tag !== null) params.set('tag', String(state.tag));
	if (state.circle !== null) params.set('circle', String(state.circle));
	if (state.va !== null) params.set('va', state.va);
	if (state.root !== null) params.set('root', state.root);
	if (state.sort !== 'added') params.set('sort', state.sort);
	// Random ignores the order, and the seed only matters for random.
	if (state.sort !== 'random' && state.order !== 'desc') params.set('order', state.order);
	if (state.sort === 'random' && state.seed !== null) params.set('seed', String(state.seed));
	if (state.page !== 1) params.set('page', String(state.page));
	return params.toString();
}

export function searchHref(pathname: string, state: SearchState): string {
	const query = searchStateToQuery(state);
	return query ? `${pathname}?${query}` : pathname;
}

export function randomSeed(random: () => number = Math.random): number {
	return Math.floor(random() * SEED_LIMIT);
}

/**
 * Applies a change from the controls. Anything other than the page number
 * changes the result set, so it goes back to page 1. Switching to random
 * picks a fresh seed, which is also how "shuffle again" works.
 */
export function updateSearchState(
	state: SearchState,
	patch: Partial<SearchState>,
	random: () => number = Math.random
): SearchState {
	const next: SearchState = { ...state, ...patch };
	if (!('page' in patch)) next.page = 1;
	const enteringRandom = next.sort === 'random' && (state.sort !== 'random' || next.seed === null);
	if (enteringRandom && !('seed' in patch)) next.seed = randomSeed(random);
	if (next.sort !== 'random') next.seed = null;
	return next;
}

/**
 * A random order without a seed cannot be paged, because the server would
 * reject it. The page replaces such a URL with one that carries a seed.
 */
export function needsSeed(state: SearchState): boolean {
	return state.sort === 'random' && state.seed === null;
}

export function hasFilters(state: SearchState): boolean {
	return (
		state.q.trim() !== '' ||
		state.tag !== null ||
		state.circle !== null ||
		state.va !== null ||
		state.root !== null
	);
}

export function toWorksQuery(state: SearchState, pageSize: number): WorksQuery {
	const q = state.q.trim();
	const query: WorksQuery = { page: state.page, page_size: pageSize };
	if (q !== '') query.q = q;
	if (state.tag !== null) query.tag = state.tag;
	if (state.circle !== null) query.circle = state.circle;
	if (state.va !== null) query.va = state.va;
	if (state.root !== null) query.root = state.root;
	if (state.sort !== 'added') query.sort = state.sort;
	if (state.sort === 'random') {
		if (state.seed !== null) query.seed = state.seed;
	} else if (state.order !== 'desc') {
		query.order = state.order;
	}
	return query;
}

/** Keeps only the sort and page parts, for the library page. */
export function sortOnly(state: SearchState): SearchState {
	const { sort, order, seed, page } = state;
	return { ...DEFAULT_SEARCH_STATE, sort, order, seed, page };
}
