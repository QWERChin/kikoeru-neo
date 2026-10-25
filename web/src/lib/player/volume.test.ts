import { describe, expect, it } from 'vitest';
import {
	DEFAULT_VOLUME,
	VOLUME_STEP,
	canSetVolume,
	clampVolume,
	isNativeVolumeChange,
	readVolume,
	volumeKeyStep,
	writeVolume,
	type VolumeStorage
} from './volume';

function memoryStorage(initial: Record<string, string> = {}): VolumeStorage & { data: Record<string, string> } {
	const data = { ...initial };
	return {
		data,
		getItem: (key) => data[key] ?? null,
		setItem: (key, value) => {
			data[key] = value;
		}
	};
}

const throwing: VolumeStorage = {
	getItem: () => {
		throw new DOMException('blocked', 'SecurityError');
	},
	setItem: () => {
		throw new DOMException('full', 'QuotaExceededError');
	}
};

describe('volume persistence', () => {
	it('reads back what was written', () => {
		const storage = memoryStorage();
		writeVolume({ volume: 0.35, muted: true }, storage);
		expect(readVolume(storage)).toEqual({ volume: 0.35, muted: true });
	});

	it('starts at full volume when nothing is saved', () => {
		expect(readVolume(memoryStorage())).toEqual(DEFAULT_VOLUME);
		expect(readVolume(null)).toEqual(DEFAULT_VOLUME);
	});

	it('ignores storage that throws', () => {
		expect(readVolume(throwing)).toEqual(DEFAULT_VOLUME);
		expect(() => writeVolume({ volume: 0.5, muted: false }, throwing)).not.toThrow();
	});

	it('falls back for each field that is malformed', () => {
		const key = 'kikoeru-neo.volume';
		expect(readVolume(memoryStorage({ [key]: 'not json' }))).toEqual(DEFAULT_VOLUME);
		expect(readVolume(memoryStorage({ [key]: '"0.4"' }))).toEqual(DEFAULT_VOLUME);
		expect(readVolume(memoryStorage({ [key]: '{"volume":"0.4","muted":true}' }))).toEqual({
			volume: 1,
			muted: true
		});
		expect(readVolume(memoryStorage({ [key]: '{"volume":7,"muted":"yes"}' }))).toEqual({
			volume: 1,
			muted: false
		});
		expect(readVolume(memoryStorage({ [key]: '{"volume":-1}' }))).toEqual({ volume: 0, muted: false });
	});

	it('clamps and rounds', () => {
		expect(clampVolume(0.7 + 0.05)).toBe(0.75);
		expect(clampVolume(Number.NaN)).toBe(1);
		expect(clampVolume(2)).toBe(1);
	});
});

describe('canSetVolume', () => {
	it('is true where the element keeps the assigned volume', () => {
		expect(canSetVolume(() => ({ volume: 1 }))).toBe(true);
	});

	it('is false where the assignment is ignored, as on iOS Safari', () => {
		const ios = {
			get volume() {
				return 1;
			},
			set volume(_value: number) {}
		};
		expect(canSetVolume(() => ios)).toBe(false);
	});

	it('is false when the element cannot be made', () => {
		expect(
			canSetVolume(() => {
				throw new Error('no audio');
			})
		).toBe(false);
	});
});

describe('volumeKeyStep', () => {
	const key = (k: string, mods: Partial<Record<'altKey' | 'ctrlKey' | 'metaKey' | 'shiftKey', boolean>> = {}) => ({
		key: k,
		altKey: false,
		ctrlKey: false,
		metaKey: false,
		shiftKey: false,
		...mods
	});

	it('maps up and down to a step', () => {
		expect(volumeKeyStep(key('ArrowUp'))).toBe(VOLUME_STEP);
		expect(volumeKeyStep(key('ArrowDown'))).toBe(-VOLUME_STEP);
	});

	it('leaves other keys and modified arrows alone', () => {
		expect(volumeKeyStep(key('ArrowLeft'))).toBeNull();
		expect(volumeKeyStep(key(' '))).toBeNull();
		expect(volumeKeyStep(key('ArrowUp', { shiftKey: true }))).toBeNull();
		expect(volumeKeyStep(key('ArrowDown', { metaKey: true }))).toBeNull();
	});
});

describe('isNativeVolumeChange', () => {
	it('keeps a change made in iOS native fullscreen', () => {
		expect(isNativeVolumeChange({ webkitDisplayingFullscreen: true }, false)).toBe(true);
	});

	it('treats a change while a new source settles as a reset, even in native fullscreen', () => {
		expect(isNativeVolumeChange({ webkitDisplayingFullscreen: true }, true)).toBe(false);
	});

	it('treats every other change as a reset to undo', () => {
		expect(isNativeVolumeChange({ webkitDisplayingFullscreen: false }, false)).toBe(false);
		expect(isNativeVolumeChange({}, false)).toBe(false);
	});
});
