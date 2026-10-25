import { describe, expect, it } from 'vitest';
import { ACCENTS, applyAccent, cachedAccent, isAccent, type AccentStorage } from './accent';

function memoryStorage(initial: Record<string, string> = {}): AccentStorage & { data: Record<string, string> } {
	const data = { ...initial };
	return {
		data,
		getItem: (key) => data[key] ?? null,
		setItem: (key, value) => {
			data[key] = value;
		}
	};
}

const throwing: AccentStorage = {
	getItem: () => {
		throw new Error('blocked');
	},
	setItem: () => {
		throw new Error('blocked');
	}
};

// The suite runs in Node, so a plain object stands in for <html>.
const fakeRoot = () => ({ dataset: {} as DOMStringMap }) as HTMLElement;

describe('accent', () => {
	it('knows the eight names of the contract and nothing else', () => {
		expect(ACCENTS).toHaveLength(8);
		for (const accent of ACCENTS) expect(isAccent(accent)).toBe(true);
		expect(isAccent('yellow')).toBe(false);
		expect(isAccent(null)).toBe(false);
	});

	it('starts blue when nothing or something unknown is remembered', () => {
		expect(cachedAccent(memoryStorage())).toBe('blue');
		expect(cachedAccent(memoryStorage({ 'kikoeru-neo.accent': '#ff0000' }))).toBe('blue');
		expect(cachedAccent(throwing)).toBe('blue');
		expect(cachedAccent(null)).toBe('blue');
	});

	it('sets data-accent and remembers it, with null meaning blue', () => {
		const root = fakeRoot();
		const storage = memoryStorage();
		applyAccent('teal', root, storage);
		expect(root.dataset.accent).toBe('teal');
		expect(cachedAccent(storage)).toBe('teal');
		applyAccent(null, root, storage);
		expect(root.dataset.accent).toBe('blue');
		expect(storage.data['kikoeru-neo.accent']).toBe('blue');
	});

	it('still applies the colour when storage refuses', () => {
		const root = fakeRoot();
		applyAccent('green', root, throwing);
		expect(root.dataset.accent).toBe('green');
	});
});
