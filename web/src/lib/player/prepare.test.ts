import { describe, expect, it, vi } from 'vitest';
import prepareProcessing from '../../../../contract/m2/prepare-processing.json';
import prepareReady from '../../../../contract/m2/prepare-ready.json';
import taskDone from '../../../../contract/m2/task-done.json';
import taskRunning from '../../../../contract/m2/task-running.json';
import { ApiError } from '../api/errors';
import type { PrepareRequest, PrepareResponse } from '../api/types';
import type { TaskSnapshot } from '../api/types';
import {
	MAX_LOST_TASK_PREPARES,
	MAX_REPREPARES,
	MAX_TOKEN_REFRESHES,
	POLL_INTERVAL_MS,
	PrepareFailedError,
	RETRY_LIMIT_MS,
	TrackLoader,
	UNREACHABLE_MESSAGE,
	abortError,
	playlistSet,
	detectCanPlay,
	failureOf,
	nextRecovery,
	resolvePlayable,
	type LoadState,
	type LoaderDeps
} from './prepare';

const ready = prepareReady as PrepareResponse;
const processing = prepareProcessing as PrepareResponse;
const running = taskRunning as TaskSnapshot;
const done = taskDone as TaskSnapshot;

/** A sleep that resolves on the next tick, or rejects once aborted. */
function instantSleep(ms: number, signal: AbortSignal): Promise<void> {
	void ms;
	return new Promise((resolve, reject) => {
		if (signal.aborted) return reject(abortError());
		setTimeout(() => (signal.aborted ? reject(abortError()) : resolve()), 0);
	});
}

describe('resolvePlayable', () => {
	const signal = new AbortController().signal;

	it('returns a ready answer without polling', async () => {
		const getTask = vi.fn();
		const prepare = vi.fn(async () => ready);
		const source = await resolvePlayable(
			102,
			{ canPlay: ['audio/mpeg'], forceFallback: false, signal },
			{ prepare, getTask, sleep: instantSleep }
		);
		expect(source).toEqual({ url: '/api/tracks/102/stream', mimeType: 'audio/mpeg' });
		expect(prepare).toHaveBeenCalledWith(102, { can_play: ['audio/mpeg'], force_fallback: false });
		expect(getTask).not.toHaveBeenCalled();
	});

	it('polls a processing task every 2 seconds until it is done', async () => {
		const answers = [running, running, done];
		const getTask = vi.fn(async () => answers.shift()!);
		const sleep = vi.fn(instantSleep);
		const statuses: unknown[] = [];
		const source = await resolvePlayable(
			104,
			{ canPlay: [], forceFallback: false, signal, onStatus: (s) => statuses.push(s) },
			{ prepare: async () => processing, getTask, sleep }
		);
		expect(source).toEqual({ url: '/api/tracks/104/playable', mimeType: 'audio/mp4' });
		expect(getTask).toHaveBeenCalledTimes(3);
		expect(getTask).toHaveBeenCalledWith(88);
		expect(sleep.mock.calls.every(([ms]) => ms === POLL_INTERVAL_MS)).toBe(true);
		expect(statuses).toEqual([
			{ state: 'preparing' },
			{ state: 'processing', taskId: 88, progress: null },
			{ state: 'processing', taskId: 88, progress: 0.42 },
			{ state: 'processing', taskId: 88, progress: 0.42 }
		]);
	});

	it('fails with the task message when the transcode fails', async () => {
		const failed = { ...running, state: 'failed', message: 'ffmpeg 退出码 1' } as TaskSnapshot;
		await expect(
			resolvePlayable(
				104,
				{ canPlay: [], forceFallback: true, signal },
				{ prepare: async () => processing, getTask: async () => failed, sleep: instantSleep }
			)
		).rejects.toEqual(new PrepareFailedError('ffmpeg 退出码 1'));
	});

	// A cancel from the task list and a server restart end the task without
	// a result too, and the end-to-end tests can only cancel.
	it.each(['cancelled', 'interrupted'] as const)('fails when the task ends %s', async (state) => {
		const ended = { ...running, state, message: null } as TaskSnapshot;
		await expect(
			resolvePlayable(
				104,
				{ canPlay: [], forceFallback: false, signal },
				{ prepare: async () => processing, getTask: async () => ended, sleep: instantSleep }
			)
		).rejects.toEqual(new PrepareFailedError('转码失败'));
	});

	// The server keeps who asked for a transcode in memory only, so after a
	// restart a user's poll answers 404 until prepare registers them again.
	it('prepares again when a restarted server no longer lists the task', async () => {
		const lost = new ApiError(404, 'not_found', '任务不存在');
		const answers: (TaskSnapshot | ApiError)[] = [running, lost, done];
		const getTask = vi.fn(async () => {
			const answer = answers.shift()!;
			if (answer instanceof ApiError) throw answer;
			return answer;
		});
		const prepare = vi.fn(async () => processing);
		const source = await resolvePlayable(
			104,
			{ canPlay: ['audio/mpeg'], forceFallback: true, signal },
			{ prepare, getTask, sleep: instantSleep }
		);
		expect(source).toEqual({ url: '/api/tracks/104/playable', mimeType: 'audio/mp4' });
		expect(prepare).toHaveBeenCalledTimes(2);
		expect(prepare).toHaveBeenLastCalledWith(104, { can_play: ['audio/mpeg'], force_fallback: true });
	});

	it('gives up when the task stays missing after preparing again', async () => {
		const lost = new ApiError(404, 'not_found', '任务不存在');
		const prepare = vi.fn(async () => processing);
		await expect(
			resolvePlayable(
				104,
				{ canPlay: [], forceFallback: false, signal },
				{
					prepare,
					getTask: async () => {
						throw lost;
					},
					sleep: instantSleep
				}
			)
		).rejects.toBe(lost);
		expect(prepare).toHaveBeenCalledTimes(1 + MAX_LOST_TASK_PREPARES);
	});

	it('stops polling when the signal aborts', async () => {
		const controller = new AbortController();
		const getTask = vi.fn(async () => {
			controller.abort();
			return running;
		});
		await expect(
			resolvePlayable(
				104,
				{ canPlay: [], forceFallback: false, signal: controller.signal },
				{ prepare: async () => processing, getTask, sleep: instantSleep }
			)
		).rejects.toMatchObject({ name: 'AbortError' });
		expect(getTask).toHaveBeenCalledOnce();
	});
});

describe('nextRecovery', () => {
	it('refreshes the media token on 401 at most twice', () => {
		expect(nextRecovery('unauthorized', { tokenRefreshes: 0, reprepares: 0, fallbackTried: false }).action).toBe(
			'refresh-token'
		);
		expect(
			nextRecovery('unauthorized', { tokenRefreshes: MAX_TOKEN_REFRESHES, reprepares: 0, fallbackTried: false })
				.action
		).toBe('fail');
	});

	it('prepares again after an evicted HLS set, twice at most', () => {
		const attempts = { tokenRefreshes: 0, reprepares: 0, fallbackTried: false };
		expect(nextRecovery('not-ready', attempts).action).toBe('reprepare');
		expect(nextRecovery('not-ready', { ...attempts, reprepares: MAX_REPREPARES }).action).toBe('fail');
	});

	it('reads 401 as an expired token and 409 as an evicted HLS set', () => {
		expect(failureOf(401)).toBe('unauthorized');
		expect(failureOf(409)).toBe('not-ready');
		expect(failureOf(200)).toBe('media');
	});

	it('reads a request that failed and a proxy without its server as an unreachable server', () => {
		expect(failureOf(0)).toBe('unreachable');
		expect(failureOf(502)).toBe('unreachable');
		expect(failureOf(504)).toBe('unreachable');
		// A 503 on a media URL is the server's own storage_unavailable.
		expect(failureOf(503)).toBe('media');
		expect(nextRecovery('unreachable', { tokenRefreshes: 0, reprepares: 0, fallbackTried: false }).action).toBe(
			'wait'
		);
	});

	it('asks for a fallback once on any other media error', () => {
		expect(nextRecovery('media', { tokenRefreshes: 0, reprepares: 0, fallbackTried: false }).action).toBe('fallback');
		expect(nextRecovery('media', { tokenRefreshes: 0, reprepares: 0, fallbackTried: true }).action).toBe('fail');
	});
});

describe('detectCanPlay', () => {
	it('keeps the types the browser answers probably or maybe for', () => {
		const answers: Record<string, string> = { 'audio/mpeg': 'probably', 'audio/flac': 'maybe' };
		expect(detectCanPlay((type) => answers[type] ?? '')).toEqual(['audio/mpeg', 'audio/flac']);
	});
});

function loaderSetup(overrides: Partial<LoaderDeps> = {}) {
	const states: LoadState[] = [];
	const sources: [string, string, number][] = [];
	const prepareCalls: [number, PrepareRequest][] = [];
	let token = 'media-1';
	const deps: LoaderDeps = {
		prepare: async (trackId, body) => {
			prepareCalls.push([trackId, body]);
			return body.force_fallback
				? { state: 'ready', url: `/api/tracks/${trackId}/playable`, mime_type: 'audio/mp4' }
				: { state: 'ready', url: `/api/tracks/${trackId}/stream`, mime_type: 'audio/mpeg' };
		},
		getTask: async () => done,
		sleep: instantSleep,
		canPlay: () => ['audio/mpeg'],
		mediaToken: async () => token,
		refreshMediaToken: async () => {
			token = `media-${Number(token.split('-')[1]) + 1}`;
			return token;
		},
		probeStatus: async () => 200,
		probePlaylist: async () => ({ status: 200, set: 'set-a' }),
		setSource: (url, mime, position) => sources.push([url, mime, position]),
		onState: (state) => states.push(state),
		now: () => 0,
		...overrides
	};
	// Unless a test says otherwise, a playlist answers like any other URL
	// and always points to the same set.
	if (!overrides.probePlaylist) {
		deps.probePlaylist = async (url) => ({ status: await deps.probeStatus(url), set: 'set-a' });
	}
	return { loader: new TrackLoader(deps), states, sources, prepareCalls };
}

describe('TrackLoader', () => {
	it('says a library is out of reach, not that the track is broken, on a 503', async () => {
		const { loader, sources, states } = loaderSetup({
			prepare: async () => {
				throw new ApiError(503, 'storage_unavailable', '媒体库 网盘 暂时无法访问');
			}
		});
		await loader.load(102, 0);
		expect(sources).toEqual([]);
		expect(states.at(-1)).toEqual({ state: 'error', message: '媒体库 网盘 暂时无法访问。恢复之后点重试。' });
	});

	it('sets the ready source with the media token and the start position', async () => {
		const { loader, sources, states } = loaderSetup();
		await loader.load(102, 812000);
		expect(sources).toEqual([['/api/tracks/102/stream?token=media-1', 'audio/mpeg', 812000]]);
		expect(states.at(-1)).toEqual({ state: 'ready' });
	});

	it('ignores a slow answer for a track the user already switched away from', async () => {
		let release: () => void = () => {};
		const slow = new Promise<void>((r) => (release = r));
		const { loader, sources } = loaderSetup({
			prepare: async (trackId) => {
				if (trackId === 101) await slow;
				return { state: 'ready', url: `/api/tracks/${trackId}/stream`, mime_type: 'audio/mpeg' };
			}
		});
		const first = loader.load(101, 0);
		await loader.load(102, 0);
		release();
		await first;
		expect(sources.map(([url]) => url)).toEqual(['/api/tracks/102/stream?token=media-1']);
	});

	it('refreshes an expired media token and reloads at the same position, twice at most', async () => {
		const { loader, sources, states } = loaderSetup({ probeStatus: async () => 401 });
		await loader.load(102, 0);
		await loader.handleMediaError(5000);
		await loader.handleMediaError(6000);
		await loader.handleMediaError(7000);
		expect(sources).toEqual([
			['/api/tracks/102/stream?token=media-1', 'audio/mpeg', 0],
			['/api/tracks/102/stream?token=media-2', 'audio/mpeg', 5000],
			['/api/tracks/102/stream?token=media-3', 'audio/mpeg', 6000]
		]);
		expect(states.at(-1)).toMatchObject({ state: 'error' });
	});

	it('probes the URL the element loaded, not one with a newer shared token', async () => {
		const probed: string[] = [];
		let token = 'media-1';
		const { loader, sources } = loaderSetup({
			mediaToken: async () => token,
			probeStatus: async (url) => {
				probed.push(url);
				return url.includes('media-1') ? 401 : 200;
			}
		});
		await loader.load(102, 0);
		// A cover refreshed the shared token in the meantime.
		token = 'media-9';
		await loader.handleMediaError(1000);
		expect(probed).toEqual(['/api/tracks/102/stream?token=media-1']);
		expect(sources.at(-1)?.[0]).toBe('/api/tracks/102/stream?token=media-2');
	});

	it('retries once with force_fallback when the browser cannot play the file', async () => {
		const { loader, sources, states, prepareCalls } = loaderSetup();
		await loader.load(104, 0);
		await loader.handleMediaError(3000);
		expect(prepareCalls.map(([, body]) => body.force_fallback)).toEqual([false, true]);
		expect(sources.at(-1)).toEqual(['/api/tracks/104/playable?token=media-1', 'audio/mp4', 3000]);
		await loader.handleMediaError(3000);
		expect(prepareCalls).toHaveLength(2);
		expect(states.at(-1)).toEqual({ state: 'error', message: '这条轨道无法播放' });
	});

	it('refreshes the token of an HLS playlist whose segments were rejected', async () => {
		const hls = { state: 'ready', url: '/api/tracks/201/hls/index.m3u8', mime_type: 'application/vnd.apple.mpegurl' } as const;
		const { loader, sources } = loaderSetup({
			prepare: async () => hls,
			probeStatus: async (url) => (url.includes('media-1') ? 401 : 200)
		});
		await loader.load(201, 0);
		await loader.handleMediaError(42_000);
		expect(sources).toEqual([
			['/api/tracks/201/hls/index.m3u8?token=media-1', 'application/vnd.apple.mpegurl', 0],
			['/api/tracks/201/hls/index.m3u8?token=media-2', 'application/vnd.apple.mpegurl', 42_000]
		]);
	});

	it('prepares again without forcing a fallback when the HLS set was evicted', async () => {
		const { loader, sources, prepareCalls } = loaderSetup({ probeStatus: async () => 409 });
		await loader.load(201, 0);
		await loader.handleMediaError(12_000);
		expect(prepareCalls.map(([, body]) => body.force_fallback)).toEqual([false, false]);
		expect(sources.at(-1)).toEqual(['/api/tracks/201/stream?token=media-1', 'audio/mpeg', 12_000]);
	});

	it('keeps the fallback when a transcoded HLS set is evicted', async () => {
		let status = 200;
		const { loader, prepareCalls } = loaderSetup({ probeStatus: async () => status });
		await loader.load(201, 0);
		await loader.handleMediaError(1000);
		status = 409;
		await loader.handleMediaError(2000);
		expect(prepareCalls.map(([, body]) => body.force_fallback)).toEqual([false, true, true]);
	});

	it('refreshes the token of a stalled source whose URL answers 401', async () => {
		const { loader, sources } = loaderSetup({ probeStatus: async () => 401 });
		await loader.load(201, 0);
		await loader.handleStall(30_000);
		expect(sources.at(-1)).toEqual(['/api/tracks/201/stream?token=media-2', 'audio/mpeg', 30_000]);
	});

	it('leaves a stalled source alone when its URL still works', async () => {
		const { loader, sources, prepareCalls } = loaderSetup({ probeStatus: async () => 200 });
		await loader.load(201, 0);
		await loader.handleStall(30_000);
		expect(sources).toHaveLength(1);
		expect(prepareCalls).toHaveLength(1);
	});

	it('ignores a probe answer that arrives after the source was recovered', async () => {
		let release: (status: number) => void = () => {};
		let calls = 0;
		const refreshes = vi.fn(async () => 'media-2');
		const { loader, sources } = loaderSetup({
			refreshMediaToken: refreshes,
			probeStatus: async () => {
				calls += 1;
				return calls === 1 ? new Promise<number>((r) => (release = r)) : 401;
			}
		});
		await loader.load(102, 0);
		const stall = loader.handleStall(10_000);
		await loader.handleMediaError(10_000);
		release(401);
		await stall;
		expect(refreshes).toHaveBeenCalledOnce();
		expect(sources).toHaveLength(2);
	});

	it('ignores a late 409 after the evicted set was already prepared again at the same URL', async () => {
		const hls = { state: 'ready', url: '/api/tracks/201/hls/index.m3u8', mime_type: 'application/vnd.apple.mpegurl' } as const;
		let release: (answer: { status: number; set: string | null }) => void = () => {};
		let calls = 0;
		const { loader, prepareCalls } = loaderSetup({
			prepare: async (trackId, body) => {
				prepareCalls.push([trackId, body]);
				return hls;
			},
			probePlaylist: async () => {
				calls += 1;
				// Call 1 records the set at load; call 2 is the slow stall probe.
				if (calls === 2) return new Promise((r) => (release = r));
				return { status: calls === 3 ? 409 : 200, set: 'set-a' };
			}
		});
		await loader.load(201, 0);
		const stall = loader.handleStall(5000);
		await loader.handleMediaError(5000);
		release({ status: 409, set: null });
		await stall;
		expect(prepareCalls).toHaveLength(2);
	});

	it('recovers once when two probes find the same evicted set before either recovery ends', async () => {
		const hls = { state: 'ready', url: '/api/tracks/201/hls/index.m3u8', mime_type: 'application/vnd.apple.mpegurl' } as const;
		let release: () => void = () => {};
		const { loader, prepareCalls, sources } = loaderSetup({
			prepare: async (trackId, body) => {
				prepareCalls.push([trackId, body]);
				// The first reprepare is still out when the second probe answers.
				if (prepareCalls.length === 2) await new Promise<void>((r) => (release = r));
				return hls;
			},
			probePlaylist: async () => ({ status: 409, set: null })
		});
		await loader.load(201, 0);
		const stall = loader.handleStall(5000);
		const error = loader.handleMediaError(5000);
		await vi.waitFor(() => expect(prepareCalls).toHaveLength(2));
		await new Promise((r) => setTimeout(r, 0));
		release();
		await Promise.all([stall, error]);
		expect(prepareCalls).toHaveLength(2);
		expect(sources).toHaveLength(2);
	});

	it('drops a probe that answers while the last allowed reload is still out', async () => {
		let release: (token: string) => void = () => {};
		let refreshes = 0;
		const { loader, sources, states } = loaderSetup({
			probeStatus: async () => 401,
			refreshMediaToken: async () => {
				refreshes += 1;
				if (refreshes < MAX_TOKEN_REFRESHES) return `media-${refreshes + 1}`;
				return new Promise<string>((r) => (release = r));
			}
		});
		await loader.load(102, 0);
		for (let i = 1; i < MAX_TOKEN_REFRESHES; i++) await loader.handleMediaError(5000);
		const last = loader.handleMediaError(5000);
		await vi.waitFor(() => expect(refreshes).toBe(MAX_TOKEN_REFRESHES));
		// Finds the same expired token, with no refresh left to spend.
		await loader.handleMediaError(5000);
		release('media-9');
		await last;
		expect(states.filter((s) => s.state === 'error')).toEqual([]);
		expect(sources.at(-1)?.[0]).toBe('/api/tracks/102/stream?token=media-9');
	});

	it('prepares again when the playlist now points to another HLS set', async () => {
		const hls = { state: 'ready', url: '/api/tracks/201/hls/index.m3u8', mime_type: 'application/vnd.apple.mpegurl' } as const;
		let set = 'set-a';
		const { loader, prepareCalls } = loaderSetup({
			prepare: async (trackId, body) => {
				prepareCalls.push([trackId, body]);
				return hls;
			},
			probePlaylist: async () => ({ status: 200, set })
		});
		await loader.load(201, 0);
		await Promise.resolve();
		set = 'set-b';
		await loader.handleStall(20_000);
		expect(prepareCalls.map(([, body]) => body.force_fallback)).toEqual([false, false]);
	});

	it('leaves a stalled HLS source alone while the playlist keeps its set', async () => {
		const hls = { state: 'ready', url: '/api/tracks/201/hls/index.m3u8', mime_type: 'application/vnd.apple.mpegurl' } as const;
		const { loader, sources } = loaderSetup({
			prepare: async () => hls,
			probePlaylist: async () => ({ status: 200, set: 'set-a' })
		});
		await loader.load(201, 0);
		await loader.handleStall(20_000);
		expect(sources).toHaveLength(1);
	});

	it('reports a failed prepare as an error state', async () => {
		const { ApiError } = await import('../api/errors');
		const { loader, states, sources } = loaderSetup({
			prepare: async () => {
				throw new ApiError(410, 'missing', '这个文件已经找不到了');
			}
		});
		await loader.load(105, 0);
		expect(sources).toEqual([]);
		expect(states.at(-1)).toEqual({ state: 'error', message: '这个文件已经找不到了' });
	});

	it('waits for a transcode and shows its progress', async () => {
		const { loader, states, sources } = loaderSetup({
			prepare: async () => processing,
			getTask: vi.fn().mockResolvedValueOnce(running).mockResolvedValueOnce(done)
		});
		await loader.load(104, 0);
		expect(states).toEqual([
			{ state: 'preparing' },
			{ state: 'processing', progress: null },
			{ state: 'processing', progress: 0.42 },
			{ state: 'ready' }
		]);
		expect(sources[0]?.[0]).toBe('/api/tracks/104/playable?token=media-1');
	});
});

describe('TrackLoader on a server out of reach', () => {
	const offline = () => new ApiError(0, 'network_error', '无法连接到服务器');
	const readyFor = (trackId: number): PrepareResponse => ({
		state: 'ready',
		url: `/api/tracks/${trackId}/stream`,
		mime_type: 'audio/mpeg'
	});
	/** A sleep that only ends when its signal aborts. */
	const endless = (_ms: number, signal: AbortSignal) =>
		new Promise<void>((_, reject) => signal.addEventListener('abort', () => reject(abortError()), { once: true }));

	it('keeps trying a prepare that cannot reach the server and plays once it can', async () => {
		const delays: number[] = [];
		let calls = 0;
		const { loader, states, sources } = loaderSetup({
			prepare: async (trackId) => {
				calls += 1;
				if (calls <= 2) throw offline();
				return readyFor(trackId);
			},
			sleep: (ms, signal) => {
				delays.push(ms);
				return instantSleep(ms, signal);
			}
		});
		await loader.load(102, 5000);
		expect(delays).toEqual([2000, 4000]);
		expect(states).toContainEqual({ state: 'error', message: UNREACHABLE_MESSAGE, retrying: true });
		expect(states.at(-1)).toEqual({ state: 'ready' });
		expect(sources).toEqual([['/api/tracks/102/stream?token=media-1', 'audio/mpeg', 5000]]);
	});

	it('retries behind a proxy whose server is down, but not on storage_unavailable from the server', async () => {
		let proxied = 0;
		const behindProxy = loaderSetup({
			prepare: async (trackId) => {
				proxied += 1;
				if (proxied === 1) throw new ApiError(502, 'bad_response', '服务器返回了无法识别的响应（HTTP 502）');
				return readyFor(trackId);
			}
		});
		await behindProxy.loader.load(102, 0);
		expect(proxied).toBe(2);
		expect(behindProxy.states.at(-1)).toEqual({ state: 'ready' });

		let stored = 0;
		const storage = loaderSetup({
			prepare: async () => {
				stored += 1;
				throw new ApiError(503, 'storage_unavailable', '媒体库 网盘 暂时无法访问');
			}
		});
		await storage.loader.load(102, 0);
		expect(stored).toBe(1);
		expect(storage.states.at(-1)).toEqual({ state: 'error', message: '媒体库 网盘 暂时无法访问。恢复之后点重试。' });
	});

	it('gives up after the retry limit and leaves the rest to 重试', async () => {
		let now = 0;
		const { loader, states, sources } = loaderSetup({
			prepare: async () => {
				throw offline();
			},
			now: () => now,
			sleep: (ms, signal) => {
				now += ms;
				return instantSleep(ms, signal);
			}
		});
		await loader.load(102, 0);
		expect(sources).toEqual([]);
		expect(states.at(-1)).toEqual({ state: 'error', message: '无法连接到服务器' });
		expect(now).toBeGreaterThanOrEqual(RETRY_LIMIT_MS);
		expect(now).toBeLessThan(RETRY_LIMIT_MS + 30_000);
	});

	it('stops waiting for a track the user switched away from', async () => {
		let tries101 = 0;
		const { loader, states, sources } = loaderSetup({
			prepare: async (trackId) => {
				if (trackId === 101) {
					tries101 += 1;
					throw offline();
				}
				return readyFor(trackId);
			},
			sleep: endless
		});
		const first = loader.load(101, 0);
		await vi.waitFor(() => expect(states.at(-1)).toMatchObject({ retrying: true }));
		await loader.load(102, 0);
		await first;
		expect(tries101).toBe(1);
		expect(sources.map(([url]) => url)).toEqual(['/api/tracks/102/stream?token=media-1']);
		expect(states.at(-1)).toEqual({ state: 'ready' });
	});

	it('tries again at once when asked instead of finishing the wait', async () => {
		let calls = 0;
		const { loader, states, sources } = loaderSetup({
			prepare: async (trackId) => {
				calls += 1;
				if (calls === 1) throw offline();
				return readyFor(trackId);
			},
			sleep: endless
		});
		const loading = loader.load(102, 0);
		await vi.waitFor(() => expect(states.at(-1)).toMatchObject({ retrying: true }));
		loader.retryNow();
		await loading;
		expect(calls).toBe(2);
		expect(sources).toHaveLength(1);
	});

	it('reloads a source that lost the server where it stopped, without a fallback', async () => {
		const statuses = [0, 0, 200];
		const { loader, sources, prepareCalls, states } = loaderSetup({
			probeStatus: async () => statuses.shift() ?? 200
		});
		await loader.load(102, 0);
		await loader.handleMediaError(5000);
		expect(prepareCalls).toHaveLength(1);
		expect(sources).toEqual([
			['/api/tracks/102/stream?token=media-1', 'audio/mpeg', 0],
			['/api/tracks/102/stream?token=media-1', 'audio/mpeg', 5000]
		]);
		expect(states.at(-1)).toEqual({ state: 'ready' });
	});

	it('refreshes the token when the server comes back to a source whose token expired meanwhile', async () => {
		const statuses = [0, 401];
		const { loader, sources, states } = loaderSetup({ probeStatus: async () => statuses.shift() ?? 200 });
		await loader.load(102, 0);
		await loader.handleMediaError(5000);
		expect(sources.at(-1)).toEqual(['/api/tracks/102/stream?token=media-2', 'audio/mpeg', 5000]);
		expect(states.at(-1)).toEqual({ state: 'ready' });
	});

	it('drops its reload when a stall recovery replaced the source during the wait', async () => {
		let mode: 'ok' | 'down' | 'expired' = 'ok';
		const { loader, sources, states } = loaderSetup({
			prepare: async () => ({
				state: 'ready',
				url: '/api/tracks/201/hls/index.m3u8',
				mime_type: 'application/vnd.apple.mpegurl'
			}),
			probePlaylist: async (url) => {
				if (mode === 'down') return { status: 0, set: null };
				if (mode === 'expired' && url.includes('media-1')) return { status: 401, set: null };
				return { status: 200, set: 'set-a' };
			},
			sleep: endless
		});
		await loader.load(201, 0);
		mode = 'down';
		const waiting = loader.handleMediaError(5000);
		await vi.waitFor(() => expect(states.at(-1)).toMatchObject({ retrying: true }));
		mode = 'expired';
		await loader.handleStall(8000);
		loader.retryNow();
		await waiting;
		expect(sources.map(([url, , position]) => [url, position])).toEqual([
			['/api/tracks/201/hls/index.m3u8?token=media-1', 0],
			['/api/tracks/201/hls/index.m3u8?token=media-2', 8000]
		]);
		expect(states.at(-1)).toEqual({ state: 'ready' });
	});

	it('gives a later outage the whole budget after a stall recovery ended the first one', async () => {
		let now = 0;
		let mode: 'ok' | 'down' | 'expired' = 'ok';
		const { loader, states } = loaderSetup({
			prepare: async () => ({
				state: 'ready',
				url: '/api/tracks/201/hls/index.m3u8',
				mime_type: 'application/vnd.apple.mpegurl'
			}),
			probePlaylist: async (url) => {
				if (mode === 'down') return { status: 0, set: null };
				if (mode === 'expired' && url.includes('media-1')) return { status: 401, set: null };
				return { status: 200, set: 'set-a' };
			},
			sleep: endless,
			now: () => now
		});
		await loader.load(201, 0);
		mode = 'down';
		const first = loader.handleMediaError(5000);
		await vi.waitFor(() => expect(states.at(-1)).toMatchObject({ retrying: true }));
		mode = 'expired';
		await loader.handleStall(8000);
		loader.retryNow();
		await first;
		// Plays for longer than the limit, then loses the server again.
		now += RETRY_LIMIT_MS + 60_000;
		mode = 'down';
		void loader.handleMediaError(700_000);
		await vi.waitFor(() => expect(states.at(-1)).toMatchObject({ retrying: true }));
	});

	it('counts only a stretch without any answer towards the retry limit', async () => {
		let now = 0;
		let prepares = 0;
		let polls = 0;
		const { loader, states, sources } = loaderSetup({
			prepare: async (trackId) => {
				prepares += 1;
				if (prepares === 1) throw offline();
				return prepares === 2 ? processing : readyFor(trackId);
			},
			getTask: async () => {
				polls += 1;
				if (polls > 1) throw offline();
				// A long transcode that answers every poll.
				now += RETRY_LIMIT_MS + 60_000;
				return running;
			},
			now: () => now
		});
		await loader.load(104, 0);
		expect(prepares).toBe(3);
		expect(sources).toHaveLength(1);
		expect(states.at(-1)).toEqual({ state: 'ready' });
	});

	it('leaves a stalled source alone while the server is out of reach', async () => {
		const { loader, sources, states } = loaderSetup({
			prepare: async () => ({ state: 'ready', url: '/api/tracks/201/hls/index.m3u8', mime_type: 'application/vnd.apple.mpegurl' }),
			probePlaylist: async () => ({ status: 0, set: null })
		});
		await loader.load(201, 0);
		await loader.handleStall(20_000);
		expect(sources).toHaveLength(1);
		expect(states.at(-1)).toEqual({ state: 'ready' });
	});
});

describe('detectCanPlay', () => {
	it('reports the codec-qualified types the server needs for Opus, Vorbis and ALAC', () => {
		const supported = new Set([
			'audio/ogg; codecs="opus"',
			'audio/ogg; codecs="vorbis"',
			'audio/webm; codecs="opus"',
			'audio/mp4; codecs="alac"'
		]);
		const types = detectCanPlay((type) => (supported.has(type) ? 'probably' : ''));
		expect(types).toEqual([
			'audio/mp4; codecs="alac"',
			'audio/ogg; codecs="vorbis"',
			'audio/ogg; codecs="opus"',
			'audio/webm; codecs="opus"'
		]);
	});
});

describe('playlistSet', () => {
	it('reads the set from the first segment URI of the sample playlist', async () => {
		const { readFileSync } = await import('node:fs');
		const text = readFileSync(new URL('../../../../contract/m6/playlist.m3u8', import.meta.url), 'utf8');
		expect(playlistSet(text)).toBe(
			'3f2a9c51e07b84d6a1c3e5f7092b4d6e8f0a1c3e5b7d9f1a2c4e6a8b0d2f4a6c-1791700300123456789'
		);
		expect(playlistSet('{"error":{}}')).toBeNull();
	});
});
