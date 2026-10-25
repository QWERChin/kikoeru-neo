// Mock state for the M4 admin endpoints that do not involve tasks: missing
// works and their cleanup, and tag merging. Loaded only in mock mode,
// through mock.ts.
import missingSample from '../../../../contract/m4/missing.json';
import type { CleanupRequest, CleanupResponse, MissingReport, TagMergeRequest, WorkSummary } from './types';
import { catalog, favoriteOrder, historyOrder, removeFrom, tags } from './mock-catalog';

const missing: MissingReport = structuredClone(missingSample);

export function getMissing(): MissingReport {
	return missing;
}

/**
 * What DELETE /admin/roots/{id} leaves behind. Every mock work has one
 * location, so the whole work goes missing, and the report lists missing
 * locations only for works that still have another.
 */
export function markRootMissing(works: Pick<WorkSummary, 'id' | 'code' | 'title'>[]): void {
	const at = Date.now();
	for (const work of works) {
		if (!missing.works.some((w) => w.id === work.id)) {
			missing.works.push({ id: work.id, code: work.code, title: work.title, missing_at: at });
		}
	}
}

/** Deletes the selected rows that are still missing, like the server. */
export function cleanup({ work_ids, location_ids }: CleanupRequest): CleanupResponse {
	const works = missing.works.filter((w) => work_ids.includes(w.id));
	const locations = missing.locations.filter((l) => location_ids.includes(l.id));
	missing.works = missing.works.filter((w) => !works.includes(w));
	missing.locations = missing.locations.filter((l) => !locations.includes(l));
	for (const work of works) {
		// The work goes with its favorites and progress (book/src/scanning.md).
		const at = catalog.findIndex((w) => w.summary.id === work.id);
		if (at >= 0) catalog.splice(at, 1);
		removeFrom(favoriteOrder, work.id);
		removeFrom(historyOrder, work.id);
	}
	return { works_deleted: works.length, locations_deleted: locations.length };
}

/** Alias names per canonical tag, so a merged-away name still finds its tag. */
const aliases = new Map<number, string[]>();

/** The canonical tag that has `name` as an alias, or null. */
export function aliasOwner(name: string): number | null {
	for (const [id, names] of aliases) if (names.includes(name)) return id;
	return null;
}

export type MergeResult = { ok: true } | { ok: false; message: string };

export function mergeTags({ from, into }: TagMergeRequest): MergeResult {
	if (from === into) return { ok: false, message: '不能把标签合并进它自己' };
	const source = tags.find((t) => t.id === from);
	const target = tags.find((t) => t.id === into);
	if (!source || !target) return { ok: false, message: '没有这个标签' };
	for (const work of catalog) {
		if (!work.tagIds.includes(from)) continue;
		work.tagIds = work.tagIds.filter((id) => id !== from);
		if (!work.tagIds.includes(into)) work.tagIds.push(into);
	}
	target.work_count += source.work_count;
	aliases.set(into, [...(aliases.get(into) ?? []), source.name, ...(aliases.get(from) ?? [])]);
	aliases.delete(from);
	tags.splice(tags.indexOf(source), 1);
	return { ok: true };
}
