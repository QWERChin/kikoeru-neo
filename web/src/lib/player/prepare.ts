import { ApiError, CLIENT_ERROR_CODES } from '../api/errors';
import { withMediaToken } from '../api/media';
import { isStorageUnavailable, storageUnavailableText } from '../storage-error';
import type { PrepareRequest, PrepareResponse } from '../api/types';
import type { TaskSnapshot } from '../api/types';
import { isHlsType } from './vidstack-source';

/** The contract asks clients to poll a transcode task every 2 seconds. */
export const POLL_INTERVAL_MS = 2000;

/**
 * Prepares again after a poll answers 404. The server keeps who asked for a
 * transcode in memory only, so after a restart prepare has to register the
 * user again (book: media.md). Once is enough for that, the second covers a
 * restart during the first.
 */
export const MAX_LOST_TASK_PREPARES = 2;

export interface PlayableSource {
	url: string;
	mimeType: string;
}

export type PrepareStatus =
	| { state: 'preparing' }
	| { state: 'processing'; taskId: number; progress: number | null };

export interface PrepareDeps {
	prepare(trackId: number, body: PrepareRequest): Promise<PrepareResponse>;
	getTask(taskId: number): Promise<TaskSnapshot>;
	/** Resolves after `ms`, or rejects with an AbortError when `signal` aborts. */
	sleep(ms: number, signal: AbortSignal): Promise<void>;
}

/** The task behind a prepare ended without a playable file. */
export class PrepareFailedError extends Error {
	constructor(message: string) {
		super(message);
		this.name = 'PrepareFailedError';
	}
}

export function abortError(): DOMException {
	return new DOMException('Playback switched to another track', 'AbortError');
}

export function isAbortError(error: unknown): boolean {
	return error instanceof DOMException && error.name === 'AbortError';
}

export function sleep(ms: number, signal: AbortSignal): Promise<void> {
	return new Promise((resolve, reject) => {
		if (signal.aborted) return reject(abortError());
		const timer = setTimeout(() => {
			signal.removeEventListener('abort', onAbort);
			resolve();
		}, ms);
		function onAbort() {
			clearTimeout(timer);
			reject(abortError());
		}
		signal.addEventListener('abort', onAbort, { once: true });
	});
}

const FAILED_STATES = new Set<TaskSnapshot['state']>(['failed', 'cancelled', 'interrupted']);

/**
 * Asks the server for a version of the track the browser can play. A ready
 * answer is returned at once. A processing answer is polled until the
 * transcode task is done or fails. Switching tracks aborts `signal`, which
 * only stops waiting here: the server keeps transcoding, so coming back to
 * the track later finds the cached file.
 */
export async function resolvePlayable(
	trackId: number,
	options: {
		canPlay: string[];
		forceFallback: boolean;
		signal: AbortSignal;
		onStatus?: (status: PrepareStatus) => void;
		/** Every answer from the server, so the loader knows it is reachable. */
		onAnswer?: () => void;
	},
	deps: PrepareDeps
): Promise<PlayableSource> {
	const { signal, onStatus, onAnswer } = options;
	const check = () => {
		if (signal.aborted) throw abortError();
	};

	for (let lost = 0; ; lost++) {
		onStatus?.({ state: 'preparing' });
		const answer = await deps.prepare(trackId, {
			can_play: options.canPlay,
			force_fallback: options.forceFallback
		});
		onAnswer?.();
		check();
		if (answer.state === 'ready') return { url: answer.url, mimeType: answer.mime_type };

		const taskId = answer.task_id;
		onStatus?.({ state: 'processing', taskId, progress: null });
		try {
			return await pollTask(taskId, signal, onStatus, onAnswer, deps);
		} catch (error) {
			check();
			const missing = error instanceof ApiError && error.status === 404;
			if (!missing || lost >= MAX_LOST_TASK_PREPARES) throw error;
		}
	}
}

async function pollTask(
	taskId: number,
	signal: AbortSignal,
	onStatus: ((status: PrepareStatus) => void) | undefined,
	onAnswer: (() => void) | undefined,
	deps: PrepareDeps
): Promise<PlayableSource> {
	for (;;) {
		await deps.sleep(POLL_INTERVAL_MS, signal);
		const task = await deps.getTask(taskId);
		onAnswer?.();
		if (signal.aborted) throw abortError();
		if (task.state === 'done') {
			if (!task.result) throw new PrepareFailedError('转码完成了，但服务器没有给出播放地址');
			return { url: task.result.url, mimeType: task.result.mime_type };
		}
		if (FAILED_STATES.has(task.state)) {
			throw new PrepareFailedError(task.message ?? '转码失败');
		}
		onStatus?.({ state: 'processing', taskId, progress: task.progress });
	}
}

/**
 * MIME types worth asking about. They mirror `mime_candidates` in
 * crates/neo-media/src/decide.rs: for Opus, Vorbis, ALAC and AAC in MP4 the
 * server only plays the original when the codec-qualified type is listed, so
 * a bare container type would make it transcode for nothing.
 */
export const CANDIDATE_TYPES = [
	'audio/mpeg',
	'audio/mp4',
	'audio/mp4; codecs="mp4a.40.2"',
	'audio/mp4; codecs="alac"',
	'audio/aac',
	'audio/wav',
	'audio/flac',
	'audio/ogg',
	'audio/ogg; codecs="vorbis"',
	'audio/ogg; codecs="opus"',
	'audio/opus',
	'audio/webm',
	'audio/webm; codecs="opus"',
	'video/mp4',
	'video/mp4; codecs="avc1.42E01E, mp4a.40.2"',
	'video/webm'
];

/**
 * `canPlayType` of a detached media element, so the answer is real even
 * before the Vidstack player has loaded. Video types need a video element.
 */
export function browserCanPlayType(type: string): string {
	if (typeof document === 'undefined') return '';
	const element = document.createElement(type.startsWith('video/') ? 'video' : 'audio');
	return element.canPlayType(type);
}

/** Types `canPlayType` answers `probably` or `maybe` for, per the contract. */
export function detectCanPlay(probe: (type: string) => string): string[] {
	return CANDIDATE_TYPES.filter((type) => probe(type) !== '');
}

export type MediaFailure = 'unauthorized' | 'not-ready' | 'unreachable' | 'media';

export type Recovery =
	| { action: 'refresh-token' }
	| { action: 'reprepare' }
	| { action: 'fallback' }
	| { action: 'wait' }
	| { action: 'fail' };

/** The waits between tries while the server is out of reach. The last one repeats. */
export const RETRY_DELAYS_MS = [2000, 4000, 8000, 15_000, 30_000] as const;
/** After this long out of reach the player stops trying and waits for 重试. */
export const RETRY_LIMIT_MS = 10 * 60_000;
export const UNREACHABLE_MESSAGE = '无法连接到服务器，连上之后会自动继续';

/**
 * What a reverse proxy answers while the server behind it is down or
 * restarting. The server's own 503 is storage_unavailable with a JSON body,
 * which keeps its manual 重试 (frontend.md), so only a 503 without one counts.
 */
const GATEWAY_STATUSES = new Set([502, 503, 504]);

/** True when the request never reached a working server. */
export function isUnreachable(error: unknown): boolean {
	if (!(error instanceof ApiError)) return false;
	if (error.code === CLIENT_ERROR_CODES.network) return true;
	return error.code === CLIENT_ERROR_CODES.badResponse && GATEWAY_STATUSES.has(error.status);
}

export function retryDelay(attempt: number): number {
	return RETRY_DELAYS_MS[Math.min(attempt, RETRY_DELAYS_MS.length - 1)]!;
}

/** Media token refreshes per track load, so a broken token cannot loop. */
export const MAX_TOKEN_REFRESHES = 2;
/** New prepares after an evicted HLS set, per track load. */
export const MAX_REPREPARES = 2;

export interface RecoveryAttempts {
	tokenRefreshes: number;
	reprepares: number;
	fallbackTried: boolean;
}

/**
 * What to do after the media element reports an error. A 401 means the
 * media token expired, so it is refreshed and the same source reloaded at
 * the same position, at most twice. A 409 means the HLS set behind the
 * playlist was evicted, so prepare runs again with the same fallback choice
 * and builds a new one. Any other failure means the browser could not play
 * the file, so the server is asked once for a transcoded fallback before
 * giving up. A server out of reach says nothing about the file, so the
 * loader waits for it instead of spending the fallback.
 */
export function nextRecovery(failure: MediaFailure, attempts: RecoveryAttempts): Recovery {
	if (failure === 'unreachable') return { action: 'wait' };
	if (failure === 'unauthorized') {
		return attempts.tokenRefreshes < MAX_TOKEN_REFRESHES
			? { action: 'refresh-token' }
			: { action: 'fail' };
	}
	if (failure === 'not-ready') {
		return attempts.reprepares < MAX_REPREPARES ? { action: 'reprepare' } : { action: 'fail' };
	}
	return attempts.fallbackTried ? { action: 'fail' } : { action: 'fallback' };
}

/**
 * How the status of a HEAD request to the failed source reads as a failure.
 * 0 is a request that failed, 502 and 504 a proxy whose server is down. A
 * 503 is left out: on a media URL it is the server's own storage_unavailable.
 */
export function failureOf(status: number): MediaFailure {
	if (status === 0 || status === 502 || status === 504) return 'unreachable';
	if (status === 401) return 'unauthorized';
	// media_not_ready: the playlist's HLS set is gone from the cache.
	if (status === 409) return 'not-ready';
	return 'media';
}

export type LoadState =
	| { state: 'idle' }
	| { state: 'preparing' }
	| { state: 'processing'; progress: number | null }
	| { state: 'ready' }
	/** `retrying` while the loader waits to try an unreachable server again. */
	| { state: 'error'; message: string; retrying?: true };

export interface LoaderDeps extends PrepareDeps {
	canPlay(): string[];
	mediaToken(): Promise<string | null>;
	refreshMediaToken(): Promise<string>;
	/** HTTP status of a HEAD request to the source, or 0 when it fails. */
	probeStatus(url: string): Promise<number>;
	/**
	 * GETs an HLS playlist: its status, and the set its segments point to
	 * (null when the answer is not a playlist).
	 */
	probePlaylist(url: string): Promise<{ status: number; set: string | null }>;
	/** Hands the source to the media element and seeks to `positionMs`. */
	setSource(url: string, mimeType: string, positionMs: number): void;
	onState(state: LoadState): void;
	now(): number;
}

interface CurrentLoad {
	trackId: number;
	controller: AbortController;
	source: PlayableSource | null;
	/** The URL with the token the element actually loaded, for the probe. */
	loadedUrl: string | null;
	/** For HLS, the set the loaded playlist pointed to. */
	hlsSet: string | null;
	/** Bumped on every setSource, even for the same URL, so stale probes can be told apart. */
	sourceGeneration: number;
	tokenRefreshes: number;
	reprepares: number;
	fallbackTried: boolean;
	/** Whether the current source came from a `force_fallback` prepare. */
	forced: boolean;
	/** When the server stopped answering, while it still does not. */
	unreachableSince: number | null;
	/** Waits so far in this stretch, which picks the next delay. */
	waits: number;
	/**
	 * A reload of this source is under way. A stall probe and an error
	 * probe can both find the same failure, and one failure gets one reload.
	 */
	recovering: boolean;
}

/**
 * A signal that aborts when either does. `dispose` takes its listeners off
 * the track's signal, which lives as long as the track and would otherwise
 * collect one per wait.
 */
function eitherSignal(a: AbortSignal, b: AbortSignal): { signal: AbortSignal; dispose: () => void } {
	const both = new AbortController();
	const abort = () => both.abort();
	const dispose = () => {
		a.removeEventListener('abort', abort);
		b.removeEventListener('abort', abort);
	};
	if (a.aborted || b.aborted) both.abort();
	else {
		a.addEventListener('abort', abort, { once: true });
		b.addEventListener('abort', abort, { once: true });
	}
	return { signal: both.signal, dispose };
}

function errorMessage(error: unknown): string {
	// The file is not gone, only out of reach, so the message says retrying
	// later will work rather than suggesting the track is broken.
	if (isStorageUnavailable(error)) return `${storageUnavailableText(error)}恢复之后点重试。`;
	return error instanceof Error && error.message ? error.message : '无法播放这条轨道';
}

/**
 * The play flow for one track at a time: prepare, wait for a transcode when
 * needed, set the source, and recover from media errors. Loading another
 * track aborts the previous one so its late answers are ignored.
 */
export class TrackLoader {
	#deps: LoaderDeps;
	#current: CurrentLoad | null = null;
	/** Ends the current wait for the server early. */
	#wake: AbortController | null = null;

	constructor(deps: LoaderDeps) {
		this.#deps = deps;
	}

	get trackId(): number | null {
		return this.#current?.trackId ?? null;
	}

	load(trackId: number, positionMs: number): Promise<void> {
		this.#current?.controller.abort();
		const current: CurrentLoad = {
			trackId,
			controller: new AbortController(),
			source: null,
			loadedUrl: null,
			hlsSet: null,
			sourceGeneration: 0,
			recovering: false,
			tokenRefreshes: 0,
			reprepares: 0,
			fallbackTried: false,
			forced: false,
			unreachableSince: null,
			waits: 0
		};
		this.#current = current;
		return this.#prepareAndSet(current, positionMs, false);
	}

	/**
	 * Tries an unreachable server again now instead of after the current
	 * wait, for example when the browser reports it is back online.
	 */
	retryNow(): void {
		this.#wake?.abort();
	}

	/** Stops waiting for the current track, for example when the queue is cleared. */
	cancel(): void {
		this.#current?.controller.abort();
		this.#current = null;
		this.#deps.onState({ state: 'idle' });
	}

	async handleMediaError(positionMs: number): Promise<void> {
		const current = this.#current;
		if (!current?.source || !current.loadedUrl) return;
		const failure = await this.#probe(current);
		if (failure) await this.#recover(current, failure, positionMs);
	}

	/**
	 * Checks a source that stopped making progress. Chromium's own HLS
	 * player waits forever after a segment answers 401 instead of reporting
	 * an error, so an expired token or an evicted set would never reach
	 * handleMediaError. Anything else is left alone: a slow network is not
	 * a reason to reload, and neither is a lost one, which the element may
	 * get over by itself. `beforeRecover` runs once a reload is certain.
	 */
	async handleStall(positionMs: number, beforeRecover: () => void = () => {}): Promise<void> {
		const current = this.#current;
		if (!current?.source || !current.loadedUrl) return;
		const failure = await this.#probe(current);
		if (!failure || failure === 'media' || failure === 'unreachable') return;
		beforeRecover();
		await this.#recover(current, failure, positionMs);
	}

	/**
	 * Asks the server about the URL the element loaded. The shared token may
	 * have been refreshed since, for a cover, and probing with it would hide
	 * that the element's own token expired. For HLS this is the playlist,
	 * which carries the same token as its segments. Its set is compared with
	 * the one loaded, because after an eviction and a rebuild the playlist
	 * answers 200 for a new set while the old segments answer 409.
	 *
	 * Returns null when the source was set again while the probe was out,
	 * even to the same URL, so a late answer never reloads a source that
	 * was already recovered.
	 */
	async #probe(current: CurrentLoad): Promise<MediaFailure | null> {
		return (await this.#check(current))?.failure ?? null;
	}

	/** #probe, plus whether the source answered as if nothing were wrong. */
	async #check(current: CurrentLoad): Promise<{ failure: MediaFailure; fine: boolean } | null> {
		const url = current.loadedUrl!;
		const generation = current.sourceGeneration;
		const loadedSet = current.hlsSet;
		let status: number;
		let failure: MediaFailure;
		if (isHlsType(current.source!.mimeType)) {
			const playlist = await this.#deps.probePlaylist(url);
			status = playlist.status;
			failure = failureOf(status);
			const replaced = status >= 200 && status < 300 && loadedSet && playlist.set && playlist.set !== loadedSet;
			if (failure === 'media' && replaced) failure = 'not-ready';
		} else {
			status = await this.#deps.probeStatus(url);
			failure = failureOf(status);
		}
		if (current !== this.#current || current.sourceGeneration !== generation) return null;
		// Any answer ends an outage, whichever recovery asked, so a later one
		// gets the whole retry budget again.
		if (failure !== 'unreachable') this.#reachable(current);
		return { failure, fine: failure === 'media' && status >= 200 && status < 300 };
	}

	async #recover(current: CurrentLoad, failure: MediaFailure, positionMs: number): Promise<void> {
		const source = current.source;
		// Another probe's reload of this source is out. It is dropped before
		// its action is chosen, because that reload may have spent the last
		// retry and this probe would then report a failure that is not one.
		if (!source || current.recovering) return;
		const recovery = nextRecovery(failure, current);
		switch (recovery.action) {
			case 'refresh-token':
				return this.#reloading(current, async () => {
					current.tokenRefreshes += 1;
					try {
						const fresh = await this.#deps.refreshMediaToken();
						if (current !== this.#current) return;
						// Also ends the waiting state when this follows an outage.
						this.#deps.onState({ state: 'ready' });
						this.#setSource(current, withMediaToken(source.url, fresh), positionMs);
					} catch (error) {
						if (current === this.#current) this.#fail(error);
					}
				});
			case 'reprepare':
				return this.#reloading(current, () => {
					current.reprepares += 1;
					return this.#prepareAndSet(current, positionMs, current.forced);
				});
			case 'fallback':
				return this.#reloading(current, () => {
					current.fallbackTried = true;
					return this.#prepareAndSet(current, positionMs, true);
				});
			case 'wait':
				// Not a reload yet: a stall probe may recover the source meanwhile.
				return this.#reloadWhenReachable(current, positionMs);
			case 'fail':
				this.#deps.onState({ state: 'error', message: '这条轨道无法播放' });
		}
	}

	/** Marks the source as being reloaded, so #recover drops other probes meanwhile. */
	async #reloading(current: CurrentLoad, reload: () => Promise<void>): Promise<void> {
		current.recovering = true;
		try {
			await reload();
		} finally {
			current.recovering = false;
		}
	}

	#setSource(current: CurrentLoad, url: string, positionMs: number): void {
		current.loadedUrl = url;
		current.hlsSet = null;
		const generation = ++current.sourceGeneration;
		if (isHlsType(current.source?.mimeType ?? '')) {
			// Remembers which set this playlist points to, for #probe.
			void this.#deps.probePlaylist(url).then(({ status, set }) => {
				const ok = status >= 200 && status < 300;
				if (current.sourceGeneration === generation && ok) current.hlsSet = set;
			});
		}
		this.#deps.setSource(url, current.source?.mimeType ?? '', positionMs);
	}

	#fail(error: unknown): void {
		this.#deps.onState({ state: 'error', message: errorMessage(error) });
	}

	/**
	 * Waits before the next try at a server that could not be reached.
	 * False when the track changed meanwhile, or when the server has been
	 * out of reach for RETRY_LIMIT_MS, which shows `error` and leaves the
	 * rest to 重试. A phone that dropped its connection keeps the queue
	 * going on its own once it is back, even while the screen is off.
	 */
	async #waitToRetry(current: CurrentLoad, error: unknown): Promise<boolean> {
		const now = this.#deps.now();
		current.unreachableSince ??= now;
		if (now - current.unreachableSince >= RETRY_LIMIT_MS) {
			this.#fail(error);
			return false;
		}
		this.#deps.onState({ state: 'error', message: UNREACHABLE_MESSAGE, retrying: true });
		const wake = new AbortController();
		this.#wake = wake;
		const either = eitherSignal(current.controller.signal, wake.signal);
		try {
			await this.#deps.sleep(retryDelay(current.waits), either.signal);
		} catch {
			// Woken by retryNow, or the track changed, which is checked below.
		} finally {
			either.dispose();
			if (this.#wake === wake) this.#wake = null;
		}
		current.waits += 1;
		return current === this.#current && !current.controller.signal.aborted;
	}

	#reachable(current: CurrentLoad): void {
		current.unreachableSince = null;
		current.waits = 0;
	}

	/**
	 * The element lost the server in the middle of a track. Once the source
	 * answers again it is loaded again at the same place. Any other answer
	 * goes through the usual recovery.
	 */
	async #reloadWhenReachable(current: CurrentLoad, positionMs: number): Promise<void> {
		const unreachable = new ApiError(0, CLIENT_ERROR_CODES.network, '无法连接到服务器');
		// A stall probe can recover the same source while this waits. Its
		// source wins, and this reload at an older position is dropped.
		const generation = current.sourceGeneration;
		for (;;) {
			if (!(await this.#waitToRetry(current, unreachable))) return;
			if (current.sourceGeneration !== generation) return;
			const result = await this.#check(current);
			if (!result) return;
			if (result.failure === 'unreachable') continue;
			this.#reachable(current);
			if (!result.fine) return this.#recover(current, result.failure, positionMs);
			// A stall probe's reload is under way and will set the source.
			if (current.recovering) return;
			this.#deps.onState({ state: 'ready' });
			this.#setSource(current, current.loadedUrl!, positionMs);
			return;
		}
	}

	async #prepareAndSet(current: CurrentLoad, positionMs: number, forceFallback: boolean): Promise<void> {
		const { signal } = current.controller;
		for (;;) {
			try {
				return await this.#prepareOnce(current, positionMs, forceFallback);
			} catch (error) {
				if (signal.aborted || isAbortError(error)) return;
				if (!isUnreachable(error)) return this.#fail(error);
				if (!(await this.#waitToRetry(current, error))) return;
			}
		}
	}

	async #prepareOnce(current: CurrentLoad, positionMs: number, forceFallback: boolean): Promise<void> {
		const { signal } = current.controller;
		const source = await resolvePlayable(
			current.trackId,
			{
				canPlay: this.#deps.canPlay(),
				forceFallback,
				signal,
				// A transcode can take longer than the retry limit, and only a
				// stretch without any answer counts towards it.
				onAnswer: () => this.#reachable(current),
				onStatus: (status) => {
					if (signal.aborted) return;
					this.#deps.onState(
						status.state === 'preparing'
							? { state: 'preparing' }
							: { state: 'processing', progress: status.progress }
					);
				}
			},
			this.#deps
		);
		const token = await this.#deps.mediaToken();
		if (signal.aborted) return;
		this.#reachable(current);
		current.source = source;
		current.forced = forceFallback;
		this.#deps.onState({ state: 'ready' });
		this.#setSource(current, withMediaToken(source.url, token), positionMs);
	}
}

/** The set id of an HLS playlist: the directory of its first segment URI. */
export function playlistSet(text: string): string | null {
	for (const line of text.split('\n')) {
		const uri = line.trim();
		if (!uri || uri.startsWith('#')) continue;
		const match = /^([^/?#]+)\/seg-\d+\.ts(?:[?#]|$)/.exec(uri);
		return match ? match[1]! : null;
	}
	return null;
}
