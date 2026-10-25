import { describe, expect, it } from 'vitest';
import { filterItems, highlight, normalizeName, similarItems, sortItems, type PickerItem } from './list-picker';

const item = (label: string, work_count = 1, value = label): PickerItem => ({ value, label, work_count });

describe('normalizeName', () => {
	it('folds full-width forms, case and spaces', () => {
		expect(normalizeName('ＡＳＭＲ')).toBe('asmr');
		expect(normalizeName('A S M R')).toBe('asmr');
		expect(normalizeName('耳　语')).toBe('耳语');
	});
});

describe('filterItems', () => {
	const items = [item('ASMR'), item('耳语'), item('耳舐め'), item('合成标签 01')];

	it('keeps everything for an empty or blank query', () => {
		expect(filterItems(items, '')).toHaveLength(4);
		expect(filterItems(items, '   ')).toHaveLength(4);
	});

	it('matches a substring after folding width, case and spaces', () => {
		expect(filterItems(items, '耳').map((i) => i.label)).toEqual(['耳语', '耳舐め']);
		expect(filterItems(items, 'ａｓｍｒ').map((i) => i.label)).toEqual(['ASMR']);
		expect(filterItems(items, '标签01').map((i) => i.label)).toEqual(['合成标签 01']);
	});
});

describe('sortItems', () => {
	const items = [item('b', 3), item('a', 3), item('c', 9)];

	it('puts the most used first, then orders by name', () => {
		expect(sortItems(items, 'count').map((i) => i.label)).toEqual(['c', 'a', 'b']);
	});

	it('orders by name, numbers by value', () => {
		const numbered = [item('标签 10'), item('标签 9'), item('标签 1')];
		expect(sortItems(numbered, 'name').map((i) => i.label)).toEqual(['标签 1', '标签 9', '标签 10']);
	});

	it('leaves the input untouched', () => {
		sortItems(items, 'count');
		expect(items.map((i) => i.label)).toEqual(['b', 'a', 'c']);
	});
});

describe('similarItems', () => {
	it('finds spellings that differ by width, case, spaces or one character', () => {
		const items = [item('ＡＳＭＲ', 2), item('耳語', 3), item('双耳', 40), item('asmr ', 1)];
		expect(similarItems('ASMR', items).map((i) => i.label)).toEqual(['ＡＳＭＲ', 'asmr ']);
		expect(similarItems('耳语', items).map((i) => i.label)).toEqual(['耳語']);
	});

	it('finds a name that contains the other', () => {
		const items = [item('耳舐め（右）', 5), item('耳かき', 9)];
		expect(similarItems('耳舐め', items).map((i) => i.label)).toEqual(['耳舐め（右）']);
	});

	it('allows two edits only in longer names', () => {
		const items = [item('バイノーラル録音'), item('ASMR')];
		expect(similarItems('バイノーラル収録', items).map((i) => i.label)).toEqual(['バイノーラル録音']);
		expect(similarItems('ASMB', [item('ASXY')])).toEqual([]);
	});

	it('does not pair unrelated single characters', () => {
		expect(similarItems('猫', [item('犬'), item('猫')]).map((i) => i.label)).toEqual(['猫']);
	});

	it('ranks closer spellings first, then the most used, and stops at the limit', () => {
		// 耳语录音 only contains the name, so it comes after one-edit 耳語 despite more works.
		const items = [item('耳语录音', 50), item('耳語', 2), item('耳语 ', 1)];
		expect(similarItems('耳语', items).map((i) => i.label)).toEqual(['耳语 ', '耳語', '耳语录音']);
		expect(similarItems('耳语', items, 1)).toHaveLength(1);
	});
});

describe('highlight', () => {
	it('splits around a case-insensitive match', () => {
		expect(highlight('Binaural ASMR', 'asmr')).toEqual({ before: 'Binaural ', match: 'ASMR', after: '' });
	});

	it('gives up on an empty query or a match only folding finds', () => {
		expect(highlight('ASMR', ' ')).toBeNull();
		expect(highlight('ＡＳＭＲ', 'asmr')).toBeNull();
	});
});
