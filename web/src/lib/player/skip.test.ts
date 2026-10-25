import { describe, expect, it } from 'vitest';
import { SEEK_CHOICES, seekChoices, seekSeconds } from './skip';

describe('seekSeconds', () => {
	it('keeps a whole number of seconds from 1 to 99', () => {
		expect(seekSeconds(1, 15)).toBe(1);
		expect(seekSeconds(10, 15)).toBe(10);
		expect(seekSeconds(99, 15)).toBe(99);
	});

	it('falls back when a server from before the setting sends none', () => {
		expect(seekSeconds(undefined, 15)).toBe(15);
		expect(seekSeconds(null, 30)).toBe(30);
	});

	it('falls back for values the server would refuse', () => {
		for (const bad of [0, 100, -5, 7.5, Number.NaN, Infinity, '10']) {
			expect(seekSeconds(bad, 30)).toBe(30);
		}
	});
});

describe('seekChoices', () => {
	it('offers the fixed list when the saved step is in it', () => {
		expect(seekChoices(15)).toEqual([...SEEK_CHOICES]);
	});

	it('adds a saved step that is not in the list, in order', () => {
		expect(seekChoices(12)).toEqual([5, 10, 12, 15, 20, 30, 45, 60, 90]);
		expect(seekChoices(99)).toEqual([...SEEK_CHOICES, 99]);
	});
});
