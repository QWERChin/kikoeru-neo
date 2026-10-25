import { describe, expect, it, vi } from 'vitest';
import { applyControl, MediaStateSync, mediaStateOf, type PlayingSource } from './media-bridge';

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

describe('MediaStateSync', () => {
	it('sends a state only when it changes', () => {
		const send = vi.fn(async () => null);
		const sync = new MediaStateSync(send);
		const state = mediaStateOf(playing);
		sync.update(state);
		sync.update({ ...state });
		sync.update({ ...state, playing: false });
		expect(send).toHaveBeenCalledTimes(2);
	});

	it('sends the same state again after a failure', async () => {
		const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
		const send = vi.fn().mockRejectedValueOnce('no window').mockResolvedValue(null);
		const sync = new MediaStateSync(send);
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
