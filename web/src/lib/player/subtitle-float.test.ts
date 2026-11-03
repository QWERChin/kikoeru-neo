import { describe, expect, it } from 'vitest';
import {
	DEFAULT_FLOAT_PREFS,
	floatLines,
	positionAfterDrag,
	readFloatPrefs,
	writeFloatPrefs
} from './subtitle-float.svelte';
import type { Cue } from './vtt';

const CUES: Cue[] = [
	{ start: 1, end: 3, text: '第一句' },
	{ start: 5, end: 8, text: '第二句' },
	{ start: 7, end: 9, text: '第三句' },
	{ start: 12, end: 14, text: '第四句' }
];

describe('floatLines', () => {
	it('shows the next line faded before the first cue', () => {
		expect(floatLines(CUES, 0)).toEqual({ current: null, spoken: false, next: '第一句' });
	});

	it('shows the line being spoken and the one after it', () => {
		expect(floatLines(CUES, 1)).toEqual({ current: '第一句', spoken: true, next: '第二句' });
	});

	it('keeps the last line faded between cues', () => {
		expect(floatLines(CUES, 4)).toEqual({ current: '第一句', spoken: false, next: '第二句' });
	});

	it('takes the cue that started last when cues overlap', () => {
		expect(floatLines(CUES, 7.5)).toEqual({ current: '第三句', spoken: true, next: '第四句' });
	});

	it('keeps the line that ended last when a long cue holds a short one', () => {
		const nested: Cue[] = [
			{ start: 0, end: 10, text: 'A' },
			{ start: 2, end: 4, text: 'B' },
			{ start: 12, end: 14, text: 'C' }
		];
		expect(floatLines(nested, 9)).toEqual({ current: 'A', spoken: true, next: 'C' });
		expect(floatLines(nested, 11)).toEqual({ current: 'A', spoken: false, next: 'C' });
	});

	it('has no next line after the last cue starts', () => {
		expect(floatLines(CUES, 13)).toEqual({ current: '第四句', spoken: true, next: null });
		expect(floatLines(CUES, 20)).toEqual({ current: '第四句', spoken: false, next: null });
	});
});

describe('positionAfterDrag', () => {
	const area = { left: 100, top: 50, width: 1000, height: 600 };

	it('turns where the card was dropped into fractions of the free space', () => {
		const card = { left: 400, top: 250, width: 400, height: 100 };
		expect(positionAfterDrag(card, area)).toEqual({ x: 0.5, y: 0.4 });
	});

	it('keeps the card inside the area when it is dropped past an edge', () => {
		expect(positionAfterDrag({ left: 0, top: 900, width: 400, height: 100 }, area)).toEqual({
			x: 0,
			y: 1
		});
	});

	it('puts a card as wide as the area at its left edge', () => {
		const card = { left: 100, top: 50, width: 1000, height: 100 };
		expect(positionAfterDrag(card, area)).toEqual({ x: 0, y: 0 });
	});
});

function memoryStorage(initial: Record<string, string> = {}) {
	const data = new Map(Object.entries(initial));
	return {
		getItem: (key: string) => data.get(key) ?? null,
		setItem: (key: string, value: string) => void data.set(key, value),
		data
	};
}

describe('float preferences', () => {
	it('starts switched on in the bottom right corner', () => {
		expect(readFloatPrefs(memoryStorage())).toEqual(DEFAULT_FLOAT_PREFS);
		expect(DEFAULT_FLOAT_PREFS).toEqual({ on: true, x: 1, y: 1 });
	});

	it('reads back what was written', () => {
		const storage = memoryStorage();
		writeFloatPrefs({ on: false, x: 0.25, y: 0 }, storage);
		expect(readFloatPrefs(storage)).toEqual({ on: false, x: 0.25, y: 0 });
	});

	it('falls back to the defaults for anything it cannot use', () => {
		const key = 'kikoeru-neo.subtitle-float';
		expect(readFloatPrefs(memoryStorage({ [key]: 'not json' }))).toEqual(DEFAULT_FLOAT_PREFS);
		expect(readFloatPrefs(memoryStorage({ [key]: '{"on":"yes","x":2,"y":-1}' }))).toEqual(
			DEFAULT_FLOAT_PREFS
		);
		const throwing = {
			getItem: () => {
				throw new Error('blocked');
			},
			setItem: () => {
				throw new Error('blocked');
			}
		};
		expect(readFloatPrefs(throwing)).toEqual(DEFAULT_FLOAT_PREFS);
		expect(() => writeFloatPrefs(DEFAULT_FLOAT_PREFS, throwing)).not.toThrow();
	});
});
