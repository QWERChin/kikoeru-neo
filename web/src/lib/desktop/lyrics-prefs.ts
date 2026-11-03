/** How the desktop lyrics window draws its text, kept in that window's storage. */
export interface LyricsPrefs {
	/** Pixel size of the first line; the second is three quarters of it. */
	size: number;
	color: LyricsColor;
}

export const LYRICS_COLORS = ['blue', 'pink', 'gold', 'white'] as const;
export type LyricsColor = (typeof LYRICS_COLORS)[number];

export const MIN_SIZE = 28;
export const MAX_SIZE = 52;
const SIZE_STEP = 4;
export const DEFAULT_LYRICS_PREFS: LyricsPrefs = { size: 40, color: 'blue' };
const PREFS_KEY = 'kikoeru-neo.desktop-lyrics';

/** A size `steps` steps larger (or smaller), kept inside what the window fits. */
export function resized(size: number, steps: number): number {
	return Math.min(MAX_SIZE, Math.max(MIN_SIZE, size + steps * SIZE_STEP));
}

export function nextColor(color: LyricsColor): LyricsColor {
	return LYRICS_COLORS[(LYRICS_COLORS.indexOf(color) + 1) % LYRICS_COLORS.length] ?? 'blue';
}

export type LyricsStorage = Pick<Storage, 'getItem' | 'setItem'>;

function browserStorage(): LyricsStorage | null {
	try {
		return typeof localStorage === 'undefined' ? null : localStorage;
	} catch {
		return null;
	}
}

// A per-window convenience like the floating card's place, so storage that
// throws or holds something else gives the defaults.
export function readLyricsPrefs(storage: LyricsStorage | null = browserStorage()): LyricsPrefs {
	let saved: Partial<Record<keyof LyricsPrefs, unknown>> | null = null;
	try {
		saved = JSON.parse(storage?.getItem(PREFS_KEY) ?? 'null');
	} catch {
		return DEFAULT_LYRICS_PREFS;
	}
	if (typeof saved !== 'object' || saved === null) return DEFAULT_LYRICS_PREFS;
	const size =
		typeof saved.size === 'number' && saved.size >= MIN_SIZE && saved.size <= MAX_SIZE
			? saved.size
			: DEFAULT_LYRICS_PREFS.size;
	const color = LYRICS_COLORS.find((c) => c === saved.color) ?? DEFAULT_LYRICS_PREFS.color;
	return { size, color };
}

export function writeLyricsPrefs(prefs: LyricsPrefs, storage: LyricsStorage | null = browserStorage()): void {
	try {
		storage?.setItem(PREFS_KEY, JSON.stringify(prefs));
	} catch {
		// See readLyricsPrefs.
	}
}
