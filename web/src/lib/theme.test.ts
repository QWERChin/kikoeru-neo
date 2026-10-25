import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'vitest';
import {
	THEME_COLORS,
	THEME_PREFERENCES,
	THEME_STORAGE_KEY,
	applyTheme,
	resolveTheme,
	storeThemePreference,
	storedThemePreference,
	type ThemeStorage
} from './theme';

function memoryStorage(initial: Record<string, string> = {}): ThemeStorage & { data: Record<string, string> } {
	const data = { ...initial };
	return {
		data,
		getItem: (key) => data[key] ?? null,
		setItem: (key, value) => {
			data[key] = value;
		}
	};
}

const refusing: ThemeStorage = {
	getItem: () => {
		throw new Error('SecurityError');
	},
	setItem: () => {
		throw new Error('QuotaExceededError');
	}
};

function fakeRoot() {
	return { dataset: {} as Record<string, string> } as unknown as HTMLElement;
}

function fakeMeta() {
	const attrs: Record<string, string> = {};
	return {
		attrs,
		setAttribute: (name: string, value: string) => {
			attrs[name] = value;
		}
	};
}

describe('stored preference', () => {
	test('is system when nothing or something unknown is stored', () => {
		expect(storedThemePreference(memoryStorage())).toBe('system');
		expect(storedThemePreference(memoryStorage({ [THEME_STORAGE_KEY]: 'sepia' }))).toBe('system');
		expect(storedThemePreference(null)).toBe('system');
	});

	test('round-trips each choice', () => {
		for (const preference of THEME_PREFERENCES) {
			const storage = memoryStorage();
			storeThemePreference(preference, storage);
			expect(storedThemePreference(storage)).toBe(preference);
		}
	});

	test('falls back to the system when storage is refused', () => {
		expect(() => storeThemePreference('dark', refusing)).not.toThrow();
		expect(storedThemePreference(refusing)).toBe('system');
	});
});

test('a fixed choice ignores the system and system follows it', () => {
	expect(resolveTheme('light', true)).toBe('light');
	expect(resolveTheme('dark', false)).toBe('dark');
	expect(resolveTheme('system', true)).toBe('dark');
	expect(resolveTheme('system', false)).toBe('light');
});

test('applying sets data-theme, the browser bar colour and the colour scheme', () => {
	const root = fakeRoot();
	const bar = fakeMeta();
	const scheme = fakeMeta();
	applyTheme('dark', root, bar as unknown as Element, scheme as unknown as Element);
	expect(root.dataset.theme).toBe('dark');
	expect(bar.attrs.content).toBe(THEME_COLORS.dark);
	expect(scheme.attrs.content).toBe('dark');
	applyTheme('light', root, bar as unknown as Element, scheme as unknown as Element);
	expect(root.dataset.theme).toBe('light');
	expect(bar.attrs.content).toBe(THEME_COLORS.light);
	expect(scheme.attrs.content).toBe('light');
});

// static/theme.js runs before the app and cannot import this module, so it
// repeats the rule. Running it here keeps the two from drifting apart.
describe('static/theme.js', () => {
	const source = readFileSync(new URL('../../static/theme.js', import.meta.url), 'utf8');

	function runScript(stored: string | null, systemDark: boolean, storage?: ThemeStorage) {
		const root = fakeRoot();
		const bar = fakeMeta();
		const scheme = fakeMeta();
		const metas: Record<string, ReturnType<typeof fakeMeta>> = {
			'meta[name="theme-color"]': bar,
			'meta[name="color-scheme"]': scheme
		};
		const run = new Function('localStorage', 'matchMedia', 'document', source);
		run(
			storage ?? memoryStorage(stored === null ? {} : { [THEME_STORAGE_KEY]: stored }),
			(query: string) => ({ matches: query === '(prefers-color-scheme: dark)' && systemDark }),
			{ documentElement: root, querySelector: (selector: string) => metas[selector] ?? null }
		);
		return {
			theme: root.dataset.theme,
			color: bar.attrs.content,
			scheme: scheme.attrs.content
		};
	}

	test.each([...THEME_PREFERENCES, null, 'sepia'])('agrees with the app for %s', (stored) => {
		const storage = memoryStorage(stored === null ? {} : { [THEME_STORAGE_KEY]: stored });
		for (const systemDark of [false, true]) {
			const expected = resolveTheme(storedThemePreference(storage), systemDark);
			expect(runScript(stored, systemDark)).toEqual({
				theme: expected,
				color: THEME_COLORS[expected],
				scheme: expected
			});
		}
	});

	test('follows the system when storage is refused', () => {
		expect(runScript(null, true, refusing).theme).toBe('dark');
	});
});
