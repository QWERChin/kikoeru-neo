import type { Accent } from './api/types';

/** The order of the swatches in settings, the iOS order of the system colours. */
export const ACCENTS: readonly Accent[] = ['blue', 'purple', 'pink', 'red', 'orange', 'green', 'teal', 'indigo'];

export const ACCENT_LABELS: Record<Accent, string> = {
	blue: '蓝色',
	purple: '紫色',
	pink: '粉色',
	red: '红色',
	orange: '橙色',
	green: '绿色',
	teal: '青色',
	indigo: '靛蓝'
};

/** The part of `localStorage` the accent uses, so tests can pass their own. */
export type AccentStorage = Pick<Storage, 'getItem' | 'setItem'>;

const STORAGE_KEY = 'kikoeru-neo.accent';

function browserStorage(): AccentStorage | null {
	try {
		return typeof localStorage === 'undefined' ? null : localStorage;
	} catch {
		return null;
	}
}

export function isAccent(value: unknown): value is Accent {
	return typeof value === 'string' && (ACCENTS as readonly string[]).includes(value);
}

/**
 * The accent this browser last showed. The settings arrive only after the
 * first requests, so without it every start would flash blue first.
 */
export function cachedAccent(storage: AccentStorage | null = browserStorage()): Accent {
	try {
		const value = storage?.getItem(STORAGE_KEY);
		return isAccent(value) ? value : 'blue';
	} catch {
		return 'blue';
	}
}

/** Sets the colours (app.css, `data-accent`) and remembers them for the next start. */
export function applyAccent(
	accent: Accent | null,
	root: HTMLElement | null = typeof document === 'undefined' ? null : document.documentElement,
	storage: AccentStorage | null = browserStorage()
): void {
	const value = accent ?? 'blue';
	if (root) root.dataset.accent = value;
	try {
		storage?.setItem(STORAGE_KEY, value);
	} catch {
		// Private windows can refuse storage. The colour still applies now.
	}
}
