import { activeCueIndex, type Cue } from './vtt';

/** The two lines the floating subtitle card shows at a moment. */
export interface FloatLines {
	/** The line being spoken, or the last one spoken, or null before the first. */
	current: string | null;
	/** False between cues, when `current` is the line already finished. */
	spoken: boolean;
	next: string | null;
}

/** Index of the first cue that starts after `time`, or `cues.length`. */
function firstAfter(cues: readonly Cue[], time: number): number {
	let low = 0;
	let high = cues.length;
	while (low < high) {
		const mid = (low + high) >> 1;
		if ((cues[mid]?.start ?? Infinity) <= time) low = mid + 1;
		else high = mid;
	}
	return low;
}

export function floatLines(cues: readonly Cue[], time: number): FloatLines {
	const active = activeCueIndex(cues, time);
	const upcoming = firstAfter(cues, time);
	const next = cues[upcoming]?.text ?? null;
	if (active >= 0) return { current: cues[active]?.text ?? null, spoken: true, next };
	// Every cue that started has ended. The one that ended last was heard
	// last; a short cue inside a longer one starts later but ends earlier.
	let last: Cue | null = null;
	for (let i = 0; i < upcoming; i++) {
		const cue = cues[i];
		if (cue && (!last || cue.end >= last.end)) last = cue;
	}
	return { current: last?.text ?? null, spoken: false, next };
}

/**
 * Where the card sits, as fractions of the free space in its area: 0 puts
 * it against the left or top edge, 1 against the right or bottom one. A
 * fraction stays valid when the window is resized.
 */
export interface FloatPosition {
	x: number;
	y: number;
}

export interface Rect {
	left: number;
	top: number;
	width: number;
	height: number;
}

const clamp01 = (n: number) => Math.min(1, Math.max(0, n));

/** The position for a card dropped at `card`, kept inside `area`. */
export function positionAfterDrag(card: Rect, area: Rect): FloatPosition {
	const freeX = area.width - card.width;
	const freeY = area.height - card.height;
	return {
		x: freeX > 0 ? clamp01((card.left - area.left) / freeX) : 0,
		y: freeY > 0 ? clamp01((card.top - area.top) / freeY) : 0
	};
}

export interface FloatPrefs extends FloatPosition {
	on: boolean;
}

export const DEFAULT_FLOAT_PREFS: FloatPrefs = { on: true, x: 1, y: 1 };
const PREFS_KEY = 'kikoeru-neo.subtitle-float';

export type FloatStorage = Pick<Storage, 'getItem' | 'setItem'>;

function browserStorage(): FloatStorage | null {
	try {
		return typeof localStorage === 'undefined' ? null : localStorage;
	} catch {
		return null;
	}
}

const isFraction = (n: unknown): n is number => typeof n === 'number' && n >= 0 && n <= 1;

// Like the video window's corner, this is a per-browser convenience, so
// storage that throws or holds something else gives the defaults.
export function readFloatPrefs(storage: FloatStorage | null = browserStorage()): FloatPrefs {
	let saved: Partial<Record<keyof FloatPrefs, unknown>> | null = null;
	try {
		saved = JSON.parse(storage?.getItem(PREFS_KEY) ?? 'null');
	} catch {
		return DEFAULT_FLOAT_PREFS;
	}
	if (typeof saved !== 'object' || saved === null) return DEFAULT_FLOAT_PREFS;
	return {
		on: typeof saved.on === 'boolean' ? saved.on : DEFAULT_FLOAT_PREFS.on,
		x: isFraction(saved.x) ? saved.x : DEFAULT_FLOAT_PREFS.x,
		y: isFraction(saved.y) ? saved.y : DEFAULT_FLOAT_PREFS.y
	};
}

export function writeFloatPrefs(prefs: FloatPrefs, storage: FloatStorage | null = browserStorage()): void {
	try {
		storage?.setItem(PREFS_KEY, JSON.stringify(prefs));
	} catch {
		// See readFloatPrefs.
	}
}

/** The floating subtitle card shown over the pages other than the player page. */
class SubtitleFloat {
	#prefs = $state<FloatPrefs>(readFloatPrefs());
	/** How far the card is being dragged from its position. */
	drag = $state<{ x: number; y: number } | null>(null);

	get on(): boolean {
		return this.#prefs.on;
	}

	get position(): FloatPosition {
		return this.#prefs;
	}

	setOn(on: boolean): void {
		this.#save({ ...this.#prefs, on });
	}

	setPosition(position: FloatPosition): void {
		this.#save({ ...this.#prefs, ...position });
	}

	#save(prefs: FloatPrefs): void {
		this.#prefs = prefs;
		writeFloatPrefs(prefs);
	}
}

export const subtitleFloat = new SubtitleFloat();
