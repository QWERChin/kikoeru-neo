import { describe, expect, it, vi } from 'vitest';
import fonts from '../../../../contract/m6/fonts.json';
import trackVideo from '../../../../contract/m6/track-video.json';
import type { TrackFont, TrackItem } from '../api/types';
import {
	AssController,
	fontUrls,
	startupDeadline,
	wantsAss,
	type AssDeps,
	type AssFonts,
	type AssState
} from './ass';

const video = trackVideo as TrackItem;
const element = () => ({}) as HTMLVideoElement;
const FALLBACK = {
	latin: 'https://neo.example/_app/immutable/assets/fallback-latin.AbC123.woff2',
	cjk: 'https://neo.example/_app/immutable/assets/fallback-cjk.AbC123.woff2'
};

function setup(overrides: Partial<AssDeps> = {}) {
	const states: AssState[] = [];
	const created: { content: string; fonts: AssFonts; destroy: ReturnType<typeof vi.fn> }[] = [];
	const fallbackFonts = vi.fn(() => FALLBACK);
	const deps: AssDeps = {
		loadSubtitle: async (id) => `[Script Info]\n; track ${id}`,
		listFonts: async () => fonts as TrackFont[],
		mediaToken: async () => 'media-1',
		refreshMediaToken: async () => 'media-2',
		probeStatus: async () => 200,
		create: async (_video, content, fonts) => {
			const renderer = { content, fonts, destroy: vi.fn() };
			created.push(renderer);
			return renderer;
		},
		fallbackFonts,
		onState: (state) => states.push(state),
		baseUrl: () => 'https://neo.example/player',
		...overrides
	};
	return { controller: new AssController(deps), states, created, fallbackFonts };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe('wantsAss', () => {
	it('is true for a video track with an ass subtitle only', () => {
		expect(wantsAss(video)).toBe(true);
		expect(wantsAss({ ...video, subtitle_formats: ['vtt'] })).toBe(false);
		expect(wantsAss({ ...video, kind: 'audio' })).toBe(false);
	});
});

describe('fontUrls', () => {
	it('adds the media token and makes the URL absolute for the worker', () => {
		expect(fontUrls(fonts as TrackFont[], 'a b', 'https://neo.example/player')).toEqual([
			'https://neo.example/api/tracks/210/stream?token=a%20b',
			'https://neo.example/api/tracks/211/stream?token=a%20b'
		]);
	});
});

describe('AssController', () => {
	it('renders the ass text with the work fonts', async () => {
		const { controller, states, created } = setup();
		controller.show(video, element());
		await flush();
		expect(created).toHaveLength(1);
		expect(created[0]!.content).toContain('track 201');
		expect(created[0]!.fonts.work[0]).toBe('https://neo.example/api/tracks/210/stream?token=media-1');
		expect(states.at(-1)).toEqual({ state: 'ready' });
	});

	it('hands JASSUB the fallback fonts apart from the work fonts', async () => {
		const { controller, created, fallbackFonts } = setup();
		controller.show(video, element());
		await flush();
		expect(fallbackFonts).toHaveBeenCalledOnce();
		expect(created[0]!.fonts.fallback).toEqual(FALLBACK);
		expect(created[0]!.fonts.work).toHaveLength(2);
		expect(created[0]!.fonts.work).not.toContain(FALLBACK.cjk);
	});

	it('never asks for the fallback fonts for audio or a WebVTT-only video', async () => {
		const { controller, created, fallbackFonts } = setup();
		controller.show({ ...video, kind: 'audio' }, element());
		controller.show({ ...video, id: 202, subtitle_formats: ['vtt'] }, element());
		controller.show(null, element());
		await flush();
		expect(created).toHaveLength(0);
		expect(fallbackFonts).not.toHaveBeenCalled();
	});

	it('does not ask for the fallback fonts when the subtitle fails to load', async () => {
		const { controller, fallbackFonts, states } = setup({
			loadSubtitle: async () => Promise.reject(new Error('gone'))
		});
		const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
		controller.show(video, element());
		await flush();
		expect(fallbackFonts).not.toHaveBeenCalled();
		expect(states.at(-1)).toEqual({ state: 'error' });
		warn.mockRestore();
	});

	it('refreshes an expired media token before handing the fonts to JASSUB', async () => {
		const probed: string[] = [];
		const { controller, created } = setup({
			probeStatus: async (url) => {
				probed.push(url);
				return url.includes('media-1') ? 401 : 200;
			}
		});
		controller.show(video, element());
		await flush();
		expect(probed).toEqual(['https://neo.example/api/tracks/210/stream?token=media-1']);
		expect(created[0]!.fonts.work).toEqual([
			'https://neo.example/api/tracks/210/stream?token=media-2',
			'https://neo.example/api/tracks/211/stream?token=media-2'
		]);
	});

	it('still renders when the font list fails', async () => {
		const { controller, created } = setup({ listFonts: async () => Promise.reject(new Error('x')) });
		controller.show(video, element());
		await flush();
		expect(created[0]!.fonts).toEqual({ work: [], fallback: FALLBACK });
	});

	it('does nothing again for the same track and element', async () => {
		const { controller, created } = setup();
		const el = element();
		controller.show(video, el);
		controller.show(video, el);
		await flush();
		expect(created).toHaveLength(1);
	});

	it('destroys an instance that finished after the track changed', async () => {
		let release: () => void = () => {};
		const slow = new Promise<void>((r) => (release = r));
		const { controller, created, states } = setup({
			loadSubtitle: async (id) => {
				if (id === 201) await slow;
				return `; track ${id}`;
			}
		});
		controller.show(video, element());
		controller.show({ ...video, id: 202 }, element());
		await flush();
		release();
		await flush();
		expect(created.map((r) => r.content)).toEqual(['; track 202']);
		expect(states.at(-1)).toEqual({ state: 'ready' });
	});

	it('destroys the renderer when the track has no ass subtitle', async () => {
		const { controller, created, states } = setup();
		controller.show(video, element());
		await flush();
		controller.show({ ...video, id: 202, subtitle_formats: ['vtt'] }, element());
		expect(created[0]!.destroy).toHaveBeenCalledOnce();
		expect(states.at(-1)).toEqual({ state: 'off' });
	});

	it('reports an error when JASSUB cannot start', async () => {
		const { controller, states } = setup({ create: async () => Promise.reject(new Error('no wasm')) });
		const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
		controller.show(video, element());
		await flush();
		expect(states.at(-1)).toEqual({ state: 'error' });
		warn.mockRestore();
	});
});

describe('startupDeadline', () => {
	it('passes a ready renderer through', async () => {
		await expect(startupDeadline(Promise.resolve('ok'), new EventTarget(), 1000)).resolves.toBe('ok');
	});

	it('rejects when the worker fails to load, even though ready never settles', async () => {
		const worker = new EventTarget();
		const result = startupDeadline(new Promise(() => {}), worker, 60_000);
		worker.dispatchEvent(new Event('error'));
		await expect(result).rejects.toThrow('worker failed');
	});

	it('rejects when startup takes too long', async () => {
		vi.useFakeTimers();
		const result = startupDeadline(new Promise(() => {}), new EventTarget(), 1000);
		const assertion = expect(result).rejects.toThrow('in time');
		await vi.advanceTimersByTimeAsync(1000);
		await assertion;
		vi.useRealTimers();
	});
});
