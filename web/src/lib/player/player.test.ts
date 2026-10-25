import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import detailSample from '../../../../contract/m2/work-detail.json';
import prepareReadyHls from '../../../../contract/m6/prepare-ready-hls.json';
import trackVideo from '../../../../contract/m6/track-video.json';
import type { TrackItem, WorkDetail } from '../api/types';
import type { PrepareResponse } from '../api/types';

// The player talks to the API through these modules; each test decides when
// prepare answers.
const api = vi.hoisted(() => ({
	prepareTrack: vi.fn(),
	getTask: vi.fn(),
	getSubtitle: vi.fn(async () => 'WEBVTT\n'),
	putProgress: vi.fn(async () => {}),
	finishWork: vi.fn(async () => {}),
	clearProgress: vi.fn(async () => {})
}));
vi.mock('../api/index', () => api);
vi.mock('../api/media', async (importOriginal) => ({
	...(await importOriginal<typeof import('../api/media')>()),
	ensureMediaToken: async () => 'media-token',
	refreshMediaToken: async () => 'media-token-2'
}));

const { Player, STALL_PROBE_MS } = await import('./player.svelte');
type MediaHost = import('./player.svelte').MediaHost;

const detail = detailSample as WorkDetail;
const tr01 = { groupPath: 'mp3/SE有', trackId: 101, positionMs: 0, resumed: false };
const tr02 = { groupPath: 'mp3/SE有', trackId: 102, positionMs: 0, resumed: false };

function fakeHost() {
	return {
		setSource: vi.fn(),
		clearSource: vi.fn(),
		play: vi.fn(async () => {}),
		pause: vi.fn(),
		seek: vi.fn(),
		setRate: vi.fn(),
		setVolume: vi.fn()
	} satisfies MediaHost;
}

function ready(trackId: number): PrepareResponse {
	return { state: 'ready', url: `/api/tracks/${trackId}/stream`, mime_type: 'audio/mpeg' };
}

/** A prepare answer the test releases by hand. */
function deferredPrepare() {
	let release: (answer: PrepareResponse) => void = () => {};
	api.prepareTrack.mockImplementationOnce(
		() => new Promise<PrepareResponse>((resolve) => (release = resolve))
	);
	return (answer: PrepareResponse) => release(answer);
}

const settle = () => new Promise((r) => setTimeout(r, 0));

describe('Player', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		api.prepareTrack.mockImplementation(async (id: number) => ready(id));
	});

	it('tries an unreachable server again at once when the browser is back online, screen off or not', async () => {
		const { ApiError } = await import('../api/errors');
		// attach() only listens where there is a document. The page is hidden,
		// as on a locked phone.
		const win = new EventTarget();
		vi.stubGlobal('window', win);
		vi.stubGlobal(
			'document',
			Object.assign(new EventTarget(), {
				visibilityState: 'hidden',
				createElement: () => ({ canPlayType: () => 'maybe' })
			})
		);
		try {
			api.prepareTrack.mockRejectedValueOnce(new ApiError(0, 'network_error', '无法连接到服务器'));
			const player = new Player({ volumeAdjustable: true, volumeStorage: null });
			const host = fakeHost();
			player.attach(host);
			player.playWork(detail, tr01);
			await vi.waitFor(() => expect(player.load).toMatchObject({ retrying: true }));
			win.dispatchEvent(new Event('online'));
			// Well before the first 2 second wait would end.
			await vi.waitFor(() => expect(host.setSource).toHaveBeenCalled(), { timeout: 500 });
			expect(player.load).toEqual({ state: 'ready' });
		} finally {
			vi.unstubAllGlobals();
		}
	});

	it('does not resume the previous track while the next one is preparing', async () => {
		const player = new Player();
		const host = fakeHost();
		player.attach(host);
		player.playWork(detail, tr01);
		await settle();
		player.onCanPlay();
		player.onPlay();
		host.play.mockClear();

		const release = deferredPrepare();
		player.next();
		player.pause();
		await player.play();
		expect(host.play).not.toHaveBeenCalled();

		release(ready(102));
		await settle();
		expect(host.setSource).toHaveBeenLastCalledWith(
			'/api/tracks/102/stream?token=media-token',
			'audio/mpeg'
		);
		player.onCanPlay();
		expect(host.play).toHaveBeenCalledOnce();
	});

	it.each(['first track', 'next track'] as const)(
		'cancels pending playback with toggle while preparing the %s',
		async (scenario) => {
			const player = new Player();
			const host = fakeHost();
			player.attach(host);
			if (scenario === 'next track') {
				player.playWork(detail, tr01);
				await settle();
				player.onCanPlay();
				player.onPlay();
			}
			host.play.mockClear();
			const release = deferredPrepare();
			if (scenario === 'first track') player.playWork(detail, tr02);
			else player.next();
			await settle();
			// Switching away from playing media emits a pause from the old source.
			if (scenario === 'next track') player.onPause();
			expect(player.paused).toBe(false);
			player.toggle();
			expect(player.paused).toBe(true);
			player.toggle();
			expect(player.paused).toBe(false);
			player.toggle();
			expect(player.paused).toBe(true);
			expect(host.play).not.toHaveBeenCalled();

			release(ready(102));
			await settle();
			expect(host.setSource).toHaveBeenLastCalledWith(
				'/api/tracks/102/stream?token=media-token',
				'audio/mpeg'
			);
			player.onPause();
			expect(player.paused).toBe(true);
			player.onCanPlay();
			expect(host.play).not.toHaveBeenCalled();
		}
	);

	it.each([false, true])(
		'keeps playback intent through automatic reconnect (resume: %s)',
		async (resume) => {
			const { ApiError } = await import('../api/errors');
			vi.useFakeTimers();
			const player = new Player();
			const host = fakeHost();
			player.attach(host);
			try {
				api.prepareTrack.mockRejectedValueOnce(new ApiError(0, 'network_error', '无法连接到服务器'));
				player.playWork(detail, tr01);
				await vi.advanceTimersByTimeAsync(0);
				expect(player.load).toMatchObject({ state: 'error', retrying: true });
				expect(player.paused).toBe(false);

				player.toggle();
				expect(player.paused).toBe(true);
				if (resume) {
					player.toggle();
					expect(player.paused).toBe(false);
				}
				await vi.advanceTimersByTimeAsync(0);
				expect(player.load).toMatchObject({ state: 'error', retrying: true });
				expect(api.prepareTrack).toHaveBeenCalledOnce();
				expect(host.setSource).not.toHaveBeenCalled();
				expect(host.play).not.toHaveBeenCalled();

				await vi.advanceTimersByTimeAsync(2000);
				expect(api.prepareTrack).toHaveBeenCalledTimes(2);
				expect(player.load).toEqual({ state: 'ready' });
				expect(host.setSource).toHaveBeenCalledExactlyOnceWith(
					'/api/tracks/101/stream?token=media-token',
					'audio/mpeg'
				);
				player.onPause();
				expect(player.paused).toBe(!resume);
				player.onCanPlay();
				expect(host.play).toHaveBeenCalledTimes(resume ? 1 : 0);
			} finally {
				player.stop();
				vi.useRealTimers();
			}
		}
	);

	it('restarts preparation when toggled after a terminal error', async () => {
		const { ApiError } = await import('../api/errors');
		api.prepareTrack.mockRejectedValueOnce(new ApiError(410, 'missing', '这个文件已经找不到了'));
		const player = new Player();
		const host = fakeHost();
		player.attach(host);
		player.playWork(detail, tr01);
		await settle();
		expect(player.load).toEqual({ state: 'error', message: '这个文件已经找不到了' });
		expect(player.paused).toBe(true);

		player.toggle();
		await settle();
		expect(api.prepareTrack).toHaveBeenCalledTimes(2);
		expect(player.load).toEqual({ state: 'ready' });
		player.onCanPlay();
		expect(host.play).toHaveBeenCalledOnce();
	});

	it('hands a source prepared before the host attached to the host', async () => {
		const player = new Player();
		player.playWork(detail, tr02);
		await settle();
		expect(player.load).toEqual({ state: 'ready' });

		const host = fakeHost();
		player.attach(host);
		expect(host.setSource).toHaveBeenCalledWith(
			'/api/tracks/102/stream?token=media-token',
			'audio/mpeg'
		);
	});

	it('skips back and forward by the steps from the settings', () => {
		const player = new Player();
		const host = fakeHost();
		player.attach(host);
		player.currentTime = 100;
		player.back();
		expect(host.seek).toHaveBeenLastCalledWith(85);
		player.forward();
		expect(host.seek).toHaveBeenLastCalledWith(115);
		player.setSeekSteps(10, 45);
		player.back();
		expect(host.seek).toHaveBeenLastCalledWith(105);
		player.forward();
		expect(host.seek).toHaveBeenLastCalledWith(150);
		// A server from before the setting sends neither.
		player.setSeekSteps(undefined, undefined);
		expect([player.seekBackSeconds, player.seekForwardSeconds]).toEqual([15, 30]);
	});

	it('keeps a seek made while the track is still preparing', async () => {
		const player = new Player();
		const host = fakeHost();
		player.attach(host);
		const release = deferredPrepare();
		player.playWork(detail, { ...tr02, positionMs: 5000 });
		player.seekTo(120);
		release(ready(102));
		await settle();
		player.onCanPlay();
		expect(host.seek).toHaveBeenCalledWith(120);
	});

	it('ignores events from the previous source while the next track prepares', async () => {
		const player = new Player();
		const host = fakeHost();
		player.attach(host);
		player.playWork(detail, tr01);
		await settle();
		const release = deferredPrepare();
		player.next();
		host.play.mockClear();
		// Tr01's element finishes loading only now.
		player.onCanPlay();
		player.onTimeUpdate(30);
		player.onEnded();
		expect(host.play).not.toHaveBeenCalled();
		expect(player.current?.id).toBe(102);
		expect(player.currentTime).toBe(0);
		release(ready(102));
		await settle();
		player.onCanPlay();
		expect(host.play).toHaveBeenCalledOnce();
	});

	it("ignores the previous source's duration while the next track prepares", async () => {
		const player = new Player();
		const host = fakeHost();
		player.attach(host);
		player.playWork(detail, tr01);
		await settle();
		const release = deferredPrepare();
		player.next();
		// Tr01 (301 s) reports its metadata only after the switch to Tr02.
		player.onDurationChange(301);
		expect(player.duration).toBe(1995);
		release(ready(102));
		await settle();
		player.onCanPlay();
		player.seekTo(812);
		expect(host.seek).toHaveBeenLastCalledWith(812);
	});

	it('keeps a seek made while recovering from a media error', async () => {
		const player = new Player();
		const host = fakeHost();
		player.attach(host);
		vi.stubGlobal('fetch', vi.fn(async () => new Response(null, { status: 200 })));
		player.playWork(detail, tr02);
		await settle();
		player.onCanPlay();
		player.onTimeUpdate(45);
		const release = deferredPrepare();
		player.onError();
		player.seekTo(120);
		await settle();
		release({ state: 'ready', url: '/api/tracks/102/playable', mime_type: 'audio/mp4' });
		await settle();
		host.seek.mockClear();
		player.onCanPlay();
		expect(host.seek).toHaveBeenCalledWith(120);
		vi.unstubAllGlobals();
	});

	it('asks with the browser capabilities even before the host attaches', async () => {
		vi.stubGlobal('document', {
			createElement: () => ({ canPlayType: (type: string) => (type === 'audio/mpeg' ? 'probably' : '') })
		});
		const player = new Player();
		player.playWork(detail, tr02);
		await settle();
		expect(api.prepareTrack).toHaveBeenCalledWith(102, {
			can_play: ['audio/mpeg'],
			force_fallback: false
		});
		vi.unstubAllGlobals();
	});

	it('starts a resumed track at the saved position', async () => {
		const player = new Player();
		const host = fakeHost();
		player.attach(host);
		player.playWork(detail, { ...tr02, positionMs: 812000, resumed: true });
		await settle();
		player.onCanPlay();
		expect(host.seek).toHaveBeenCalledWith(812);
	});

	describe('a pause from outside the app', () => {
		// The element's own pause event, without Player.pause(): iOS fullscreen
		// controls, unplugged headphones or a phone call.
		async function pausedThenRecovered({ pauseOutside }: { pauseOutside: boolean }) {
			vi.stubGlobal('fetch', vi.fn(async () => new Response(null, { status: 401 })));
			api.prepareTrack.mockImplementation(async () => ready(101));
			const player = new Player();
			const host = fakeHost();
			player.attach(host);
			player.playWork(detail, tr01);
			await settle();
			player.onCanPlay();
			player.onPlay();
			player.onTimeUpdate(42);
			if (pauseOutside) player.onPause();
			// A seek into unbuffered audio after the media token expired.
			host.setSource.mockClear();
			player.onError();
			await settle();
			await settle();
			expect(host.setSource).toHaveBeenCalledOnce();
			host.play.mockClear();
			player.onCanPlay();
			return host;
		}

		afterEach(() => vi.unstubAllGlobals());

		it('keeps the media paused after a recovery reload', async () => {
			const host = await pausedThenRecovered({ pauseOutside: true });
			expect(host.play).not.toHaveBeenCalled();
		});

		it('lets playing media carry on after a recovery reload', async () => {
			const host = await pausedThenRecovered({ pauseOutside: false });
			expect(host.play).toHaveBeenCalledOnce();
		});

		it("does not count the previous track's pause while the next one loads", async () => {
			const player = new Player();
			const host = fakeHost();
			player.attach(host);
			player.playWork(detail, tr01);
			await settle();
			player.onCanPlay();
			player.onPlay();
			const release = deferredPrepare();
			player.next();
			// The element reports the pause #startTrack asked for.
			player.onPause();
			expect(player.paused).toBe(false);
			release(ready(102));
			await settle();
			host.play.mockClear();
			player.onCanPlay();
			expect(host.play).toHaveBeenCalledOnce();
		});

		it('does not count the pause a new source causes before it can play', async () => {
			const player = new Player();
			const host = fakeHost();
			player.attach(host);
			player.playWork(detail, tr01);
			await settle();
			// Safari pauses the element when the source is replaced.
			player.onPause();
			expect(player.paused).toBe(false);
			host.play.mockClear();
			player.onCanPlay();
			expect(host.play).toHaveBeenCalledOnce();
		});
	});

	describe('a stalled HLS source', () => {
		const withVideo: WorkDetail = { ...detail, tracks: [...detail.tracks, trackVideo as TrackItem] };
		const video = { groupPath: '映像', trackId: 201, positionMs: 0, resumed: false };

		async function playing(fetchStatus: () => number) {
			vi.useFakeTimers();
			const fetchMock = vi.fn(async () => new Response(null, { status: fetchStatus() }));
			vi.stubGlobal('fetch', fetchMock);
			api.prepareTrack.mockImplementation(async () => prepareReadyHls as PrepareResponse);
			const player = new Player();
			const host = fakeHost();
			player.attach(host);
			player.playWork(withVideo, video);
			await vi.advanceTimersByTimeAsync(0);
			player.onCanPlay();
			player.onPlay();
			player.onTimeUpdate(42);
			return { player, host, fetchMock };
		}

		afterEach(() => {
			vi.useRealTimers();
			vi.unstubAllGlobals();
		});

		it('reloads with a fresh token at the same position when the playlist answers 401', async () => {
			const { player, host, fetchMock } = await playing(() => 401);
			player.onWaiting();
			await vi.advanceTimersByTimeAsync(STALL_PROBE_MS);
			expect(fetchMock).toHaveBeenCalledWith(
				'/api/tracks/201/hls/index.m3u8?token=media-token',
				{ cache: 'no-store' }
			);
			expect(host.setSource).toHaveBeenLastCalledWith(
				'/api/tracks/201/hls/index.m3u8?token=media-token-2',
				'application/vnd.apple.mpegurl'
			);
			host.seek.mockClear();
			player.onCanPlay();
			expect(host.seek).toHaveBeenCalledWith(42);
		});

		it('keeps a seek made while an evicted set is prepared again', async () => {
			const { player, host } = await playing(() => 409);
			const release = deferredPrepare();
			player.onWaiting();
			await vi.advanceTimersByTimeAsync(STALL_PROBE_MS);
			player.seekTo(90);
			release(prepareReadyHls as PrepareResponse);
			await vi.advanceTimersByTimeAsync(0);
			host.seek.mockClear();
			player.onCanPlay();
			expect(host.seek).toHaveBeenCalledWith(90);
		});

		it('does not probe when data arrives in time', async () => {
			const { player, fetchMock } = await playing(() => 401);
			fetchMock.mockClear();
			player.onWaiting();
			await vi.advanceTimersByTimeAsync(STALL_PROBE_MS - 1);
			player.onPlaying();
			await vi.advanceTimersByTimeAsync(STALL_PROBE_MS);
			expect(fetchMock).not.toHaveBeenCalled();
		});

		it('keeps probing a slow but valid source without reloading it', async () => {
			const { player, host, fetchMock } = await playing(() => 200);
			host.setSource.mockClear();
			fetchMock.mockClear();
			player.onWaiting();
			await vi.advanceTimersByTimeAsync(STALL_PROBE_MS * 2);
			expect(fetchMock).toHaveBeenCalledTimes(2);
			expect(host.setSource).not.toHaveBeenCalled();
		});

		it('leaves audio alone', async () => {
			vi.useFakeTimers();
			const fetchMock = vi.fn();
			vi.stubGlobal('fetch', fetchMock);
			const player = new Player();
			player.attach(fakeHost());
			player.playWork(detail, tr01);
			await vi.advanceTimersByTimeAsync(0);
			player.onCanPlay();
			player.onWaiting();
			await vi.advanceTimersByTimeAsync(STALL_PROBE_MS * 2);
			expect(fetchMock).not.toHaveBeenCalled();
		});
	});

	describe('volume', () => {
		const KEY = 'kikoeru-neo.volume';

		function memoryStorage(saved?: object) {
			const data: Record<string, string> = saved ? { [KEY]: JSON.stringify(saved) } : {};
			return {
				data,
				getItem: (key: string) => data[key] ?? null,
				setItem: (key: string, value: string) => {
					data[key] = value;
				}
			};
		}

		/** Each setSource call followed by the volume it was given right after. */
		function sourcesWithVolume(host: ReturnType<typeof fakeHost>) {
			const order = [...host.setSource.mock.invocationCallOrder].map((at) =>
				host.setVolume.mock.invocationCallOrder.findIndex((v) => v > at)
			);
			return order.map((i) => (i < 0 ? null : host.setVolume.mock.calls[i]));
		}

		afterEach(() => vi.unstubAllGlobals());

		it('restores the saved volume and mute when it starts and when a host attaches', () => {
			const player = new Player({ volumeStorage: memoryStorage({ volume: 0.3, muted: true }) });
			expect(player.volume).toBe(0.3);
			expect(player.muted).toBe(true);
			const host = fakeHost();
			player.attach(host);
			expect(host.setVolume).toHaveBeenCalledWith(0.3, true);
		});

		it('saves every change, and turning the volume up unmutes', () => {
			const storage = memoryStorage();
			const player = new Player({ volumeStorage: storage });
			const host = fakeHost();
			player.attach(host);
			player.setVolume(0.4);
			expect(JSON.parse(storage.data[KEY]!)).toEqual({ volume: 0.4, muted: false });
			player.toggleMute();
			expect(JSON.parse(storage.data[KEY]!)).toEqual({ volume: 0.4, muted: true });
			expect(host.setVolume).toHaveBeenLastCalledWith(0.4, true);
			// Turning a muted player down keeps the volume it comes back to.
			player.changeVolume(-0.05);
			expect(player.volume).toBe(0.4);
			player.changeVolume(0.05);
			expect(player.muted).toBe(false);
			expect(player.volume).toBe(0.45);
			// A later page load starts from what was saved.
			expect(new Player({ volumeStorage: storage }).volume).toBe(0.45);
		});

		it('keeps a change made with the browser controls without sending it back', () => {
			const storage = memoryStorage();
			const player = new Player({ volumeStorage: storage });
			const host = fakeHost();
			player.attach(host);
			host.setVolume.mockClear();
			player.adoptVolume(0.6, true);
			expect(player.muted).toBe(true);
			expect(JSON.parse(storage.data[KEY]!)).toEqual({ volume: 0.6, muted: true });
			expect(host.setVolume).not.toHaveBeenCalled();
		});

		it('unmuting a player turned down to 0 makes it audible again', () => {
			const player = new Player({ volumeStorage: memoryStorage() });
			player.setVolume(0);
			player.toggleMute();
			expect(player.muted).toBe(false);
			expect(player.volume).toBe(0.5);
		});

		it('keeps working when storage throws', () => {
			const broken = {
				getItem: () => {
					throw new Error('blocked');
				},
				setItem: () => {
					throw new Error('blocked');
				}
			};
			const player = new Player({ volumeStorage: broken });
			expect(player.volume).toBe(1);
			expect(() => player.setVolume(0.2)).not.toThrow();
			expect(player.volume).toBe(0.2);
		});

		it('leaves the volume to the hardware where it cannot be set, but still mutes', () => {
			const storage = memoryStorage({ volume: 0.3, muted: false });
			const player = new Player({ volumeStorage: storage, volumeAdjustable: false });
			expect(player.volume).toBe(1);
			player.setVolume(0.2);
			expect(player.volume).toBe(1);
			player.toggleMute();
			expect(player.muted).toBe(true);
			expect(JSON.parse(storage.data[KEY]!)).toEqual({ volume: 1, muted: true });
		});

		it('gives every new source the volume, the reload after a media error included', async () => {
			const player = new Player({ volumeStorage: memoryStorage({ volume: 0.6, muted: false }) });
			const host = fakeHost();
			player.attach(host);
			vi.stubGlobal('fetch', vi.fn(async () => new Response(null, { status: 200 })));
			player.playWork(detail, tr01);
			await settle();
			player.onCanPlay();
			player.setVolume(0.25);
			player.next();
			await settle();
			player.onCanPlay();
			player.onTimeUpdate(30);
			player.onError();
			await settle();
			await settle();
			expect(host.setSource).toHaveBeenCalledTimes(3);
			expect(sourcesWithVolume(host)).toEqual([
				[0.6, false],
				[0.25, false],
				[0.25, false]
			]);
		});

		it('gives a reloaded HLS source the volume too', async () => {
			vi.useFakeTimers();
			vi.stubGlobal('fetch', vi.fn(async () => new Response(null, { status: 401 })));
			api.prepareTrack.mockImplementation(async () => prepareReadyHls as PrepareResponse);
			const withVideo: WorkDetail = { ...detail, tracks: [...detail.tracks, trackVideo as TrackItem] };
			const player = new Player({ volumeStorage: memoryStorage({ volume: 0.7, muted: true }) });
			const host = fakeHost();
			player.attach(host);
			player.playWork(withVideo, { groupPath: '映像', trackId: 201, positionMs: 0, resumed: false });
			await vi.advanceTimersByTimeAsync(0);
			player.onCanPlay();
			player.onPlay();
			player.onWaiting();
			await vi.advanceTimersByTimeAsync(STALL_PROBE_MS);
			vi.useRealTimers();
			expect(host.setSource.mock.calls.length).toBeGreaterThan(1);
			expect(sourcesWithVolume(host).every((v) => v?.[0] === 0.7 && v[1] === true)).toBe(true);
		});
	});
});
