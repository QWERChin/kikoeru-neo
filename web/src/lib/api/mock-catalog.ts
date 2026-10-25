// The works the mock serves and the M3 list logic over them: search,
// filters, sorts, favorites and history. The contract samples come first,
// and synthetic works follow so paging, filters and sorts have something to
// work on. Loaded only in mock mode, through mock.ts.
import worksPage from '../../../../contract/m1/works-page.json';
import favoritesPage from '../../../../contract/m3/favorites-page.json';
import historyPage from '../../../../contract/m3/history-page.json';
import circles from '../../../../contract/m3/circles.json';
import tags from '../../../../contract/m3/tags.json';
import vas from '../../../../contract/m3/vas.json';
import type { CircleRef, Page, Progress, TagRef, VaRef, WorkSummary } from './types';
import { WORK_SORTS, type WorkSort } from './query';

export interface MockWork {
	summary: WorkSummary;
	tagIds: number[];
	vaIds: string[];
	root: string;
	/** Larger is newer, for `sort=added`. */
	added: number;
}

/** Work ids of the synthetic works, after the sample ids. */
export const SYNTHETIC_FIRST_ID = 100;
export const SYNTHETIC_COUNT = 90;
/**
 * Synthetic tags on the synthetic works, so tag lists are long enough to need
 * searching, plus two other spellings of the sample tags for the merge
 * form's near-duplicate list.
 */
export const SYNTHETIC_TAG_FIRST_ID = 2000;
export const SYNTHETIC_TAG_COUNT = 60;
export const VARIANT_TAGS = [
	{ id: 2100, name: '耳語', every: 29 },
	{ id: 2101, name: 'ＡＳＭＲ', every: 31 }
];
/** Searching for this text answers 503 storage_unavailable. */
export const MOCK_STORAGE_DOWN_QUERY = '503';
/** The library that the mock reports as unreachable. */
export const MOCK_DOWN_ROOT = { id: 'cloud', name: '云端作品' };

const SAMPLE_PROGRESS_TRACK = 345;
const TRACK_ID_STRIDE = 1000;
const SAMPLE_WORK_ID = 12;

const pad = (n: number, width = 2) => String(n).padStart(width, '0');

function syntheticCircle(id: number): CircleRef | null {
	if (id % 17 === 0) return null;
	const [first, second] = circles;
	const circle = id % 3 === 0 ? second : first;
	return circle ? { id: circle.id, name: circle.name } : null;
}

function syntheticProgress(id: number): Progress | null {
	const finished = id % 13 === 0;
	if (id % 11 !== 0 && !finished) return null;
	const trackId = SAMPLE_PROGRESS_TRACK + (id - SAMPLE_WORK_ID) * TRACK_ID_STRIDE;
	const duration = 1_995_000;
	return {
		track_id: trackId,
		position_ms: finished ? duration : Math.round(duration * ((id % 7) + 1) / 8),
		track_duration_ms: duration,
		finished,
		replaying: false,
		resumable: true,
		group_path: 'mp3/SE有'
	};
}

function syntheticWork(n: number): MockWork {
	const id = SYNTHETIC_FIRST_ID + n;
	const ratingKnown = id % 6 !== 0;
	return {
		summary: {
			id,
			code: `RJ09${pad(id, 5)}`,
			title: `合成作品 ${pad(n + 1, 3)}`,
			circle: syntheticCircle(id),
			release_date: id % 7 === 0 ? null : `202${id % 5}-${pad(1 + (id % 12))}-${pad(1 + (id % 28))}`,
			nsfw: false,
			rate_average: ratingKnown ? 3 + (id % 20) / 10 : null,
			rate_count: ratingKnown ? 10 + id : null,
			has_cover: id % 8 !== 0,
			cover_version: 1,
			missing: false,
			favorite: false,
			progress: syntheticProgress(id),
			my_rating: null
		},
		tagIds: [
			id % 2 === 0 && tags[0]?.id,
			id % 3 === 0 && tags[1]?.id,
			SYNTHETIC_TAG_FIRST_ID + (id % SYNTHETIC_TAG_COUNT),
			...VARIANT_TAGS.filter((v) => id % v.every === 0).map((v) => v.id)
		].filter((t): t is number => typeof t === 'number'),
		vaIds: [id % 2 === 1 && vas[0]?.id, id % 4 === 0 && vas[1]?.id].filter(
			(v): v is string => typeof v === 'string'
		),
		root: id % 5 === 0 ? MOCK_DOWN_ROOT.id : 'local',
		added: id
	};
}

// Tags, voice actors and libraries of the sample works, which the summaries
// do not carry. Work 12 matches contract/m2/work-detail.json.
const SAMPLE_EXTRAS: Record<number, Pick<MockWork, 'tagIds' | 'vaIds' | 'root'>> = {
	12: { tagIds: [7, 9], vaIds: ['va-sample-1', 'va-sample-2'], root: 'local' },
	13: { tagIds: [9], vaIds: ['va-sample-2'], root: MOCK_DOWN_ROOT.id },
	14: { tagIds: [7], vaIds: [], root: 'local' },
	15: { tagIds: [7], vaIds: ['va-sample-1'], root: 'local' }
};

function buildCatalog(): MockWork[] {
	const samples = [...(worksPage.items as WorkSummary[])];
	// The favorites sample has a missing work the works page cannot show.
	for (const item of favoritesPage.items as WorkSummary[]) {
		if (!samples.some((w) => w.id === item.id)) samples.push(item);
	}
	const sampleWorks = samples.map((summary, i) => ({
		summary,
		...(SAMPLE_EXTRAS[summary.id] ?? { tagIds: [], vaIds: [], root: 'local' }),
		added: 10_000 - i
	}));
	const synthetic = Array.from({ length: SYNTHETIC_COUNT }, (_, n) => syntheticWork(n));
	return [...sampleWorks, ...synthetic];
}

export const catalog: MockWork[] = buildCatalog();

const countOf = (tagId: number) => catalog.filter((w) => w.tagIds.includes(tagId)).length;
tags.push(
	...Array.from({ length: SYNTHETIC_TAG_COUNT }, (_, n) => ({
		id: SYNTHETIC_TAG_FIRST_ID + n,
		name: `合成标签 ${pad(n + 1)}`,
		work_count: countOf(SYNTHETIC_TAG_FIRST_ID + n)
	})),
	...VARIANT_TAGS.map((v) => ({ id: v.id, name: v.name, work_count: countOf(v.id) }))
);

export function tagRefs(work: MockWork): TagRef[] {
	return tags.filter((t) => work.tagIds.includes(t.id)).map(({ id, name }) => ({ id, name }));
}

export function vaRefs(work: MockWork): VaRef[] {
	return vas.filter((v) => work.vaIds.includes(v.id)).map(({ id, name }) => ({ id, name }));
}

/** Favorites newest first. Starts with the sample's order. */
export const favoriteOrder: number[] = [
	...(favoritesPage.items as WorkSummary[]).map((w) => w.id),
	...catalog
		.filter((w) => w.summary.favorite && !favoritesPage.items.some((f) => f.id === w.summary.id))
		.map((w) => w.summary.id)
];

/** Works with progress, most recently played first. Starts with the sample's order. */
export const historyOrder: number[] = [
	...(historyPage.items as WorkSummary[]).map((w) => w.id),
	...catalog
		.filter((w) => w.summary.progress !== null && !historyPage.items.some((h) => h.id === w.summary.id))
		.map((w) => w.summary.id)
];

export function moveToFront(list: number[], id: number): void {
	const at = list.indexOf(id);
	if (at >= 0) list.splice(at, 1);
	list.unshift(id);
}

export function removeFrom(list: number[], id: number): void {
	const at = list.indexOf(id);
	if (at >= 0) list.splice(at, 1);
}

export interface PageParams {
	page: number;
	pageSize: number;
}

export function pageParams(query: Record<string, unknown> | undefined): PageParams {
	const page = Math.max(1, Number(query?.page ?? 1) || 1);
	const pageSize = Math.min(100, Math.max(1, Number(query?.page_size ?? 40) || 40));
	return { page, pageSize };
}

export function paginate<T>(items: T[], { page, pageSize }: PageParams): Page<T> {
	const start = (page - 1) * pageSize;
	return { items: items.slice(start, start + pageSize), page, page_size: pageSize, total: items.length };
}

// ASCII letters only, like the server (book/src/contract.md).
const fold = (s: string) => s.replace(/[A-Z]/g, (c) => c.toLowerCase());

function matchesText(work: MockWork, needle: string): boolean {
	const { code, title, circle } = work.summary;
	const haystacks = [code, title, circle?.name ?? '', ...vaRefs(work).map((v) => v.name)];
	return haystacks.some((h) => fold(h).includes(needle));
}

// A 32-bit mix of (seed, id), so a seed gives the same order on every page.
function shuffleKey(seed: number, id: number): number {
	let h = Math.imul(seed ^ 0x9e3779b9, 0x85ebca6b) ^ Math.imul(id, 0xc2b2ae35);
	h = Math.imul(h ^ (h >>> 16), 0x7feb352d);
	return (h ^ (h >>> 15)) >>> 0;
}

export type WorksQueryResult = { ok: true; works: MockWork[] } | { ok: false; message: string };

function positiveInt(value: unknown): number | null | undefined {
	if (value === undefined) return undefined;
	const n = Number(value);
	return /^\d+$/.test(String(value)) && Number.isSafeInteger(n) && n > 0 ? n : null;
}

/** Applies the M3 search, filter and sort parameters of `GET /api/works`. */
export function queryWorks(
	works: MockWork[],
	query: Record<string, unknown> | undefined
): WorksQueryResult {
	const q = fold(String(query?.q ?? '').trim());
	const tag = positiveInt(query?.tag);
	const circle = positiveInt(query?.circle);
	const sort = String(query?.sort ?? 'added') as WorkSort;
	const order = String(query?.order ?? 'desc');
	const seed = query?.seed === undefined ? null : Number(query.seed);
	if (tag === null || circle === null) return { ok: false, message: '筛选参数必须是正整数' };
	if (!WORK_SORTS.includes(sort)) return { ok: false, message: `不认识的排序 ${sort}` };
	if (order !== 'asc' && order !== 'desc') return { ok: false, message: `不认识的顺序 ${order}` };
	if (sort === 'random' && (seed === null || !Number.isSafeInteger(seed) || seed < 0)) {
		return { ok: false, message: '随机排序需要 seed' };
	}

	const includeMissing = String(query?.include_missing) === 'true';
	const va = query?.va === undefined ? undefined : String(query.va);
	const root = query?.root === undefined ? undefined : String(query.root);
	const matched = works.filter(
		(w) =>
			(includeMissing || !w.summary.missing) &&
			(q === '' || matchesText(w, q)) &&
			(tag === undefined || w.tagIds.includes(tag)) &&
			(circle === undefined || w.summary.circle?.id === circle) &&
			(va === undefined || w.vaIds.includes(va)) &&
			(root === undefined || w.root === root)
	);

	const direction = order === 'asc' ? 1 : -1;
	const byId = (a: MockWork, b: MockWork) => direction * (a.summary.id - b.summary.id);
	// Works without the sorted value go last in both directions.
	const nullsLast = (x: string | number | null, y: string | number | null) => {
		if (x === null || y === null) return x === y ? 0 : x === null ? 1 : -1;
		return x < y ? -direction : x > y ? direction : 0;
	};
	const compare: Record<WorkSort, (a: MockWork, b: MockWork) => number> = {
		added: (a, b) => direction * (a.added - b.added) || byId(a, b),
		release: (a, b) => nullsLast(a.summary.release_date, b.summary.release_date) || byId(a, b),
		rating: (a, b) => nullsLast(a.summary.rate_average, b.summary.rate_average) || byId(a, b),
		random: (a, b) =>
			shuffleKey(seed ?? 0, a.summary.id) - shuffleKey(seed ?? 0, b.summary.id) ||
			a.summary.id - b.summary.id
	};
	return { ok: true, works: matched.sort(compare[sort]) };
}

export { circles, tags, vas };
