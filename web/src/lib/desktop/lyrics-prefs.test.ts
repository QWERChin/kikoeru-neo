import { describe, expect, it } from 'vitest';
import {
	DEFAULT_LYRICS_PREFS,
	MAX_SIZE,
	MIN_SIZE,
	nextColor,
	readLyricsPrefs,
	resized,
	writeLyricsPrefs
} from './lyrics-prefs';

function memoryStorage(initial: Record<string, string> = {}) {
	const data = new Map(Object.entries(initial));
	return {
		getItem: (key: string) => data.get(key) ?? null,
		setItem: (key: string, value: string) => void data.set(key, value)
	};
}

describe('lyrics text', () => {
	it('grows and shrinks in steps inside the window', () => {
		expect(resized(40, 1)).toBe(44);
		expect(resized(40, -1)).toBe(36);
		expect(resized(MAX_SIZE, 1)).toBe(MAX_SIZE);
		expect(resized(MIN_SIZE, -1)).toBe(MIN_SIZE);
	});

	it('cycles through the colours', () => {
		expect(nextColor('blue')).toBe('pink');
		expect(nextColor('white')).toBe('blue');
	});
});

describe('lyrics preferences', () => {
	it('reads back what was written', () => {
		const storage = memoryStorage();
		writeLyricsPrefs({ size: 32, color: 'gold' }, storage);
		expect(readLyricsPrefs(storage)).toEqual({ size: 32, color: 'gold' });
	});

	it('falls back to the defaults for anything it cannot use', () => {
		const key = 'kikoeru-neo.desktop-lyrics';
		expect(readLyricsPrefs(memoryStorage())).toEqual(DEFAULT_LYRICS_PREFS);
		expect(readLyricsPrefs(memoryStorage({ [key]: '{' }))).toEqual(DEFAULT_LYRICS_PREFS);
		expect(readLyricsPrefs(memoryStorage({ [key]: '{"size":200,"color":"red"}' }))).toEqual(
			DEFAULT_LYRICS_PREFS
		);
		const throwing = {
			getItem: () => {
				throw new Error('blocked');
			},
			setItem: () => {
				throw new Error('blocked');
			}
		};
		expect(readLyricsPrefs(throwing)).toEqual(DEFAULT_LYRICS_PREFS);
		expect(() => writeLyricsPrefs(DEFAULT_LYRICS_PREFS, throwing)).not.toThrow();
	});
});
