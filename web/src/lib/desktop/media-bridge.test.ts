import { describe, expect, it, vi } from 'vitest';
import { applyControl, lyricsOf, ShellSync, mediaStateOf, type LyricsSource, type PlayingSource } from './media-bridge';

const playing: PlayingSource = {
	active: true,
	currentTitle: '01 はじまり',
	paused: false,
	work: { circle: { name: '示例社团' } }
};

describe('mediaStateOf', () => {
	it('names the track and circle like the Media Session', () => {
		expect(mediaStateOf(playing)).toEqual({ title: '01 はじまり', artist: '示例社团', playing: true });
		expect(mediaStateOf({ ...playing, paused: true, work: { circle: null } })).toEqual({
			title: '01 はじまり',
			artist: null,
			playing: false
		});
	});

	it('clears the tray when nothing is loaded', () => {
		expect(mediaStateOf({ ...playing, active: false })).toEqual({ title: null, artist: null, playing: false });
	});
});

describe('lyricsOf', () => {
	const audio: LyricsSource = {
		...playing,
		current: { kind: 'audio' },
		currentTime: 5,
		subtitle: {
			state: 'ready',
			cues: [
				{ start: 4, end: 6, text: '第一句' },
				{ start: 8, end: 9, text: '第二句' }
			]
		}
	};

	it('sends the lines of the floating card with the title', () => {
		expect(lyricsOf(audio)).toEqual({
			current: '第一句',
			spoken: true,
			next: '第二句',
			title: '01 はじまり',
			playing: true
		});
	});

	it('sends only the title for a video or a track without lines', () => {
		const none = { current: null, spoken: false, next: null, title: '01 はじまり', playing: false };
		expect(lyricsOf({ ...audio, paused: true, current: { kind: 'video' } })).toEqual(none);
		expect(lyricsOf({ ...audio, paused: true, subtitle: { state: 'loading' } })).toEqual(none);
	});

	it('sends nothing when nothing is loaded', () => {
		expect(lyricsOf({ ...audio, active: false })).toEqual({
			current: null,
			spoken: false,
			next: null,
			title: null,
			playing: false
		});
	});
});

describe('ShellSync', () => {
	it('sends a state only when it changes', () => {
		const send = vi.fn(async () => null);
		const sync = new ShellSync(send);
		const state = mediaStateOf(playing);
		sync.update(state);
		sync.update({ ...state });
		sync.update({ ...state, playing: false });
		expect(send).toHaveBeenCalledTimes(2);
	});

	it('sends the same state again after a failure', async () => {
		const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
		const send = vi.fn().mockRejectedValueOnce('no window').mockResolvedValue(null);
		const sync = new ShellSync(send);
		const state = mediaStateOf(playing);
		sync.update(state);
		await vi.waitFor(() => expect(warn).toHaveBeenCalledOnce());
		sync.update(state);
		expect(send).toHaveBeenCalledTimes(2);
		warn.mockRestore();
	});
});

describe('applyControl', () => {
	it('runs each tray action on the player', () => {
		const player = { play: vi.fn(), pause: vi.fn(), toggle: vi.fn(), next: vi.fn(), previous: vi.fn() };
		for (const control of ['play', 'pause', 'toggle', 'next', 'previous'] as const) {
			applyControl(control, player);
			expect(player[control]).toHaveBeenCalledOnce();
		}
	});
});
