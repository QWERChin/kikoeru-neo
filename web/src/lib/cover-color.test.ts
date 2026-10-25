import { describe, expect, it } from 'vitest';
import { coverHue } from './cover-color';

describe('coverHue', () => {
	it('returns the same hue for the same id', () => {
		expect(coverHue(12)).toBe(coverHue(12));
	});

	it('gives neighbouring ids different hues', () => {
		expect(coverHue(12)).not.toBe(coverHue(13));
	});

	it('stays on the hue wheel', () => {
		for (const id of [1, 2, 2 ** 31, 2 ** 53 - 1]) {
			const hue = coverHue(id);
			expect(hue).toBeGreaterThanOrEqual(0);
			expect(hue).toBeLessThan(360);
		}
	});
});
