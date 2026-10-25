import { describe, expect, it, vi } from 'vitest';
import trackVideo from '../../../../contract/m6/track-video.json';
import type { TrackItem } from '../api/types';
import { nearestCorner, presentation, readCorner, slotStyle, videoAspect, videoView, writeCorner } from './video.svelte';

describe('presentation', () => {
	const base = {
		isVideo: true,
		fullscreen: false,
		onPlayerPage: false,
		ready: true,
		pictureInPicture: false,
		floatClosed: false,
		hasThumb: false
	};

	it('keeps audio hidden wherever the user is', () => {
		expect(presentation({ ...base, isVideo: false, onPlayerPage: true })).toBe('hidden');
		expect(presentation({ ...base, isVideo: false, fullscreen: true })).toBe('hidden');
		expect(presentation({ ...base, isVideo: false })).toBe('hidden');
	});

	it('draws a video over the player page slot once it is ready', () => {
		expect(presentation({ ...base, onPlayerPage: true })).toBe('slot');
		expect(presentation({ ...base, onPlayerPage: true, ready: false })).toBe('hidden');
	});

	it('floats a video on the other pages until the window is closed', () => {
		expect(presentation(base)).toBe('float');
		expect(presentation({ ...base, ready: false })).toBe('hidden');
		expect(presentation({ ...base, floatClosed: true, hasThumb: true })).toBe('thumb');
		expect(presentation({ ...base, floatClosed: true })).toBe('hidden');
	});

	it('leaves the picture to the browser in picture-in-picture', () => {
		expect(presentation({ ...base, pictureInPicture: true })).toBe('hidden');
	});

	it('shows a fullscreen video even after the slot went away', () => {
		expect(presentation({ ...base, fullscreen: true })).toBe('fullscreen');
	});
});

describe('nearestCorner', () => {
	const area = { left: 0, top: 0, right: 1000, bottom: 800 };
	it('picks the quarter of the area the centre is in', () => {
		expect(nearestCorner({ x: 100, y: 100 }, area)).toBe('top-left');
		expect(nearestCorner({ x: 900, y: 100 }, area)).toBe('top-right');
		expect(nearestCorner({ x: 100, y: 700 }, area)).toBe('bottom-left');
		expect(nearestCorner({ x: 600, y: 500 }, area)).toBe('bottom-right');
	});

	it('divides by the area, which the sidebar narrows, not the viewport', () => {
		// An 800px window: the area starts after the 16rem sidebar.
		const narrowed = { left: 272, top: 16, right: 784, bottom: 812 };
		// A left window's centre, dragged 8px right, is still nearer the left.
		expect(nearestCorner({ x: 480, y: 700 }, narrowed)).toBe('bottom-left');
		expect(nearestCorner({ x: 540, y: 700 }, narrowed)).toBe('bottom-right');
	});
});

describe('corner storage', () => {
	it('reads back a saved corner and falls back otherwise', () => {
		const data: Record<string, string> = {};
		const storage = { getItem: (k: string) => data[k] ?? null, setItem: (k: string, v: string) => void (data[k] = v) };
		expect(readCorner(storage)).toBe('bottom-right');
		writeCorner('top-left', storage);
		expect(readCorner(storage)).toBe('top-left');
		data['kikoeru-neo.video-corner'] = 'middle';
		expect(readCorner(storage)).toBe('bottom-right');
	});

	it('survives storage that throws', () => {
		const broken = {
			getItem: () => {
				throw new Error('blocked');
			},
			setItem: () => {
				throw new Error('blocked');
			}
		};
		expect(readCorner(broken)).toBe('bottom-right');
		expect(() => writeCorner('top-left', broken)).not.toThrow();
	});
});

describe('slotStyle', () => {
	it('pins the element to the slot box', () => {
		expect(slotStyle({ top: 10.123, left: 0, width: 320, height: 180 })).toBe(
			'top:10.12px;left:0px;width:320px;height:180px'
		);
	});
});

describe('videoAspect', () => {
	it('uses the probed size and falls back to 16:9', () => {
		expect(videoAspect(trackVideo as TrackItem)).toBeCloseTo(16 / 9);
		expect(videoAspect({ video: { codec: 'h264', width: 1080, height: 1920 } })).toBeCloseTo(9 / 16);
		expect(videoAspect({ video: { codec: 'h264', width: null, height: null } })).toBeCloseTo(16 / 9);
		expect(videoAspect({ video: null })).toBeCloseTo(16 / 9);
		expect(videoAspect(null)).toBeCloseTo(16 / 9);
	});
});

describe('videoView', () => {
	it('forwards fullscreen and picture-in-picture to the connected host only', async () => {
		const controls = { toggleFullscreen: vi.fn(async () => {}), togglePictureInPicture: vi.fn(async () => {}) };
		const disconnect = videoView.connect(controls);
		await videoView.toggleFullscreen();
		await videoView.togglePictureInPicture();
		disconnect();
		await videoView.toggleFullscreen();
		expect(controls.toggleFullscreen).toHaveBeenCalledOnce();
		expect(controls.togglePictureInPicture).toHaveBeenCalledOnce();
	});

	it('forgets a slot only when the same element detaches', () => {
		const a = {} as HTMLElement;
		const b = {} as HTMLElement;
		const detachA = videoView.attachSlot(a);
		const detachB = videoView.attachSlot(b);
		detachA();
		expect(videoView.slot).toBe(b);
		detachB();
		expect(videoView.slot).toBeNull();
	});
});
