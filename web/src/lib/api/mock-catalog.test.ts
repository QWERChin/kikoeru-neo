import { describe, expect, it } from 'vitest';
import { SYNTHETIC_COUNT, catalog, paginate, queryWorks } from './mock-catalog';

function ids(query: Record<string, unknown>): number[] {
	const result = queryWorks(catalog, query);
	if (!result.ok) throw new Error(result.message);
	return result.works.map((w) => w.summary.id);
}

describe('mock works query', () => {
	it('lists the samples first and hides missing works by default', () => {
		const all = ids({});
		expect(all.slice(0, 3)).toEqual([12, 13, 14]);
		expect(all).not.toContain(15);
		expect(all).toHaveLength(3 + SYNTHETIC_COUNT);
		expect(ids({ include_missing: 'true' })).toContain(15);
	});

	it('matches text in code, title, circle and voice actor, ignoring ASCII case', () => {
		expect(ids({ q: 'rj01234567' })).toEqual([12]);
		expect(ids({ q: '示例声优乙' })).toContain(13);
		expect(ids({ q: '  ' })).toHaveLength(3 + SYNTHETIC_COUNT);
	});

	it('keeps works without a release date last in both directions', () => {
		const desc = ids({ sort: 'release' });
		const asc = ids({ sort: 'release', order: 'asc' });
		const undated = catalog.filter((w) => w.summary.release_date === null && !w.summary.missing).length;
		expect(new Set(desc.slice(-undated))).toEqual(new Set(asc.slice(-undated)));
	});

	it('gives the same random order for the same seed, so pages do not overlap', () => {
		const a = ids({ sort: 'random', seed: 7 });
		expect(ids({ sort: 'random', seed: 7 })).toEqual(a);
		expect(ids({ sort: 'random', seed: 8 })).not.toEqual(a);
		const pages = [1, 2, 3].flatMap((page) => paginate(a, { page, pageSize: 40 }).items);
		expect(new Set(pages).size).toBe(a.length);
	});

	it('rejects what the server rejects', () => {
		expect(queryWorks(catalog, { sort: 'random' }).ok).toBe(false);
		expect(queryWorks(catalog, { sort: 'popular' }).ok).toBe(false);
		expect(queryWorks(catalog, { tag: 'x' }).ok).toBe(false);
	});

	it('filters by tag, circle, voice actor and library together', () => {
		const result = queryWorks(catalog, { tag: 7, circle: 3, va: 'va-sample-1', root: 'local' });
		expect(result.ok && result.works.length).toBeGreaterThan(0);
		for (const w of result.ok ? result.works : []) {
			expect(w.tagIds).toContain(7);
			expect(w.summary.circle?.id).toBe(3);
			expect(w.vaIds).toContain('va-sample-1');
			expect(w.root).toBe('local');
		}
	});
});
