// Searching, ordering and near-duplicate detection for ListPicker, kept out
// of the component so they can be tested on their own.

/** One choice in a ListPicker: a tag, circle or voice actor. */
export interface PickerItem {
	value: string;
	label: string;
	work_count: number;
}

export type PickerOrder = 'count' | 'name';

const collator = new Intl.Collator('zh-CN', { numeric: true });

/**
 * Folds full-width forms, case and spaces, so ＡＳＭＲ, asmr and "A S M R"
 * all compare equal. Names come from DLsite, old imports and manual edits,
 * which disagree on exactly these.
 */
export function normalizeName(name: string): string {
	return name.normalize('NFKC').toLowerCase().replace(/\s+/gu, '');
}

export function filterItems(items: readonly PickerItem[], query: string): PickerItem[] {
	const needle = normalizeName(query);
	if (needle === '') return [...items];
	return items.filter((item) => normalizeName(item.label).includes(needle));
}

function byName(a: PickerItem, b: PickerItem): number {
	return collator.compare(a.label, b.label) || a.value.localeCompare(b.value);
}

export function sortItems(items: readonly PickerItem[], order: PickerOrder): PickerItem[] {
	const sorted = [...items];
	sorted.sort(order === 'count' ? (a, b) => b.work_count - a.work_count || byName(a, b) : byName);
	return sorted;
}

/** Levenshtein distance by code point, giving up once it exceeds `limit`. */
function distance(a: string[], b: string[], limit: number): number {
	if (Math.abs(a.length - b.length) > limit) return limit + 1;
	let previous = Array.from({ length: b.length + 1 }, (_, j) => j);
	for (let i = 1; i <= a.length; i++) {
		const current = [i];
		let best = i;
		for (let j = 1; j <= b.length; j++) {
			const cost = a[i - 1] === b[j - 1] ? 0 : 1;
			current[j] = Math.min(previous[j]! + 1, current[j - 1]! + 1, previous[j - 1]! + cost);
			best = Math.min(best, current[j]!);
		}
		if (best > limit) return limit + 1;
		previous = current;
	}
	return previous[b.length]!;
}

/**
 * Items whose name is probably another spelling of `name`: equal once width,
 * case and spaces are folded, one character apart (a traditional or variant
 * form, 耳语 and 耳語), two apart in longer names, or one containing the
 * other. Closest first, then the most used. Single characters only match
 * exactly, since any two would be one edit apart.
 */
export function similarItems(name: string, items: readonly PickerItem[], limit = 8): PickerItem[] {
	const target = [...normalizeName(name)];
	const found: { item: PickerItem; rank: number }[] = [];
	for (const item of items) {
		const other = [...normalizeName(item.label)];
		const shorter = Math.min(target.length, other.length);
		const allowed = shorter >= 6 ? 2 : shorter >= 2 ? 1 : 0;
		const edits = distance(target, other, allowed);
		if (edits <= allowed) {
			found.push({ item, rank: edits });
			continue;
		}
		const contains = target.join('').includes(other.join('')) || other.join('').includes(target.join(''));
		if (shorter >= 2 && contains) found.push({ item, rank: allowed + 1 });
	}
	found.sort((a, b) => a.rank - b.rank || b.item.work_count - a.item.work_count || byName(a.item, b.item));
	return found.slice(0, limit).map((f) => f.item);
}

/**
 * Splits a label around the first case-insensitive occurrence of the query,
 * for highlighting. A match that only exists after folding width or spaces
 * is not highlighted, since its position in the label is not known.
 */
export function highlight(label: string, query: string): { before: string; match: string; after: string } | null {
	const needle = query.trim().toLowerCase();
	if (needle === '') return null;
	const at = label.toLowerCase().indexOf(needle);
	if (at < 0) return null;
	return { before: label.slice(0, at), match: label.slice(at, at + needle.length), after: label.slice(at + needle.length) };
}
