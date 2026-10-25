/** Volume and mute as the player keeps them, and as they are saved. */
export interface VolumeSetting {
	/** From 0 to 1. */
	volume: number;
	muted: boolean;
}

export const DEFAULT_VOLUME: VolumeSetting = { volume: 1, muted: false };

const STORAGE_KEY = 'kikoeru-neo.volume';

/** One arrow key press moves the volume by this much. */
export const VOLUME_STEP = 0.05;

/** The part of `localStorage` the volume uses, so tests can pass their own. */
export type VolumeStorage = Pick<Storage, 'getItem' | 'setItem'>;

function browserStorage(): VolumeStorage | null {
	try {
		return typeof localStorage === 'undefined' ? null : localStorage;
	} catch {
		// Reading the property itself throws when site data is blocked.
		return null;
	}
}

export function clampVolume(volume: number): number {
	if (!Number.isFinite(volume)) return DEFAULT_VOLUME.volume;
	// Rounded so that repeated steps do not drift to 0.7500000000000001.
	return Math.round(Math.min(1, Math.max(0, volume)) * 100) / 100;
}

// The volume is a per-browser convenience. Storage can throw in private
// windows or hold anything an older version wrote, and then the player
// starts at full volume, unmuted.
export function readVolume(storage: VolumeStorage | null = browserStorage()): VolumeSetting {
	try {
		const parsed: unknown = JSON.parse(storage?.getItem(STORAGE_KEY) ?? 'null');
		if (typeof parsed !== 'object' || parsed === null) return { ...DEFAULT_VOLUME };
		const { volume, muted } = parsed as Record<string, unknown>;
		return {
			volume: typeof volume === 'number' ? clampVolume(volume) : DEFAULT_VOLUME.volume,
			muted: muted === true
		};
	} catch {
		return { ...DEFAULT_VOLUME };
	}
}

export function writeVolume(setting: VolumeSetting, storage: VolumeStorage | null = browserStorage()): void {
	try {
		storage?.setItem(STORAGE_KEY, JSON.stringify({ volume: clampVolume(setting.volume), muted: setting.muted }));
	} catch {
		// See readVolume.
	}
}

/** The part of a media element the volume check needs. */
export interface VolumeProbe {
	volume: number;
}

/**
 * iOS Safari leaves the volume to the hardware buttons: assigning
 * `volume` is ignored and it keeps reading 1. Asking the element is the
 * only check that also covers iPads that report a desktop user agent.
 */
export function canSetVolume(create: () => VolumeProbe): boolean {
	try {
		const probe = create();
		probe.volume = 0.5;
		return Math.abs(probe.volume - 0.5) < 0.01;
	} catch {
		return false;
	}
}

export interface VolumeKey {
	key: string;
	altKey: boolean;
	ctrlKey: boolean;
	metaKey: boolean;
	shiftKey: boolean;
}

/**
 * The volume change an arrow key asks for, or null. Left and right are
 * left alone because the progress slider uses them, and keys with a
 * modifier belong to the browser.
 */
export function volumeKeyStep(event: VolumeKey): number | null {
	if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return null;
	if (event.key === 'ArrowUp') return VOLUME_STEP;
	if (event.key === 'ArrowDown') return -VOLUME_STEP;
	return null;
}

/** The part of a media element that tells whether its own controls are showing. */
export interface NativeControlsProbe {
	webkitDisplayingFullscreen?: boolean;
}

/**
 * True when a volume change on the element came from the browser's own
 * controls, which only show in iOS Safari's native video fullscreen. That
 * change is the user's and is kept. Until a new source plays, Vidstack
 * resets the volume to its own defaults even in native fullscreen, so
 * changes while `settling` are resets, as is every change elsewhere, and
 * the player's volume is put back.
 */
export function isNativeVolumeChange(media: NativeControlsProbe, settling: boolean): boolean {
	return !settling && media.webkitDisplayingFullscreen === true;
}
