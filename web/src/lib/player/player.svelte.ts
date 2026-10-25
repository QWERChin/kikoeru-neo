import {
	clearProgress,
	finishWork,
	getSubtitle,
	getTask,
	prepareTrack,
	putProgress
} from '../api/index';
import { coverUrl, ensureMediaToken, refreshMediaToken } from '../api/media';
import type { TrackItem, WorkDetail } from '../api/types';
import { session } from '../stores/session.svelte';
import { installHandlers, setMetadata, setPositionState } from './media-session';
import {
	TrackLoader,
	browserCanPlayType,
	detectCanPlay,
	playlistSet,
	sleep,
	type LoadState
} from './prepare';
import { ProgressReporter, type ProgressSnapshot } from './progress-reporter';
import { buildQueue, groupOf, isPlayable, trackTitle, type StartPoint } from './queue';
import { DEFAULT_SEEK_BACK_SECONDS, DEFAULT_SEEK_FORWARD_SECONDS, seekSeconds } from './skip';
import { isHlsType } from './vidstack-source';
import { activeCueIndex, parseVtt, type Cue } from './vtt';
import { canSetVolume, clampVolume, readVolume, writeVolume, type VolumeStorage } from './volume';

export const PLAYBACK_RATES = [0.75, 1, 1.25, 1.5, 2] as const;

export type SleepMode = 'off' | 15 | 30 | 60 | 'end-of-track';
export const SLEEP_MODES: SleepMode[] = ['off', 15, 30, 60, 'end-of-track'];

/** Going back within the first seconds of a track jumps to the previous one. */
const RESTART_THRESHOLD_SECONDS = 3;

/** Unmuting a player whose volume was dragged to 0 brings it back to this. */
const UNMUTE_VOLUME = 0.5;

/** How long an HLS source may wait for data before its playlist is probed. */
export const STALL_PROBE_MS = 6000;

/** The part of the media element the player drives. VidstackHost implements it. */
export interface MediaHost {
	setSource(url: string, mimeType: string): void;
	clearSource(): void;
	play(): Promise<void>;
	pause(): void;
	seek(seconds: number): void;
	setRate(rate: number): void;
	setVolume(volume: number, muted: boolean): void;
}

export interface PlayerOptions {
	/** Where volume and mute are saved. Defaults to `localStorage`. */
	volumeStorage?: VolumeStorage | null;
	/** Whether the page may set the volume. Defaults to asking an audio element. */
	volumeAdjustable?: boolean;
}

export type PlayerWork = Pick<
	WorkDetail,
	'id' | 'code' | 'title' | 'circle' | 'has_cover' | 'cover_version'
>;

export type SubtitleState =
	| { state: 'none' }
	| { state: 'loading' }
	| { state: 'ready'; cues: Cue[] }
	| { state: 'error' };

export class Player {
	work = $state<PlayerWork | null>(null);
	queue = $state<TrackItem[]>([]);
	groupPath = $state('');
	index = $state(0);
	load = $state<LoadState>({ state: 'idle' });
	/** Bumped after every progress write that reached the server. */
	savedRevision = $state(0);
	/** What the last write in `savedRevision` was. */
	lastSavedKind: 'put' | 'finish' | 'clear' = 'put';
	currentTime = $state(0);
	duration = $state(0);
	rate = $state(1);
	volume = $state(1);
	muted = $state(false);
	/** False on iOS Safari, where only the hardware buttons change the volume. */
	volumeAdjustable = $state(true);
	sleepMode = $state<SleepMode>('off');
	sleepEndsAt = $state<number | null>(null);
	subtitle = $state<SubtitleState>({ state: 'none' });
	/** How far back and forward skip, from the user's settings (PlaybackExtras). */
	seekBackSeconds = $state(DEFAULT_SEEK_BACK_SECONDS);
	seekForwardSeconds = $state(DEFAULT_SEEK_FORWARD_SECONDS);

	current = $derived<TrackItem | null>(this.queue[this.index] ?? null);
	currentTitle = $derived(this.current ? trackTitle(this.current.path) : '');
	activeCue = $derived(
		this.subtitle.state === 'ready' ? activeCueIndex(this.subtitle.cues, this.currentTime) : -1
	);
	hasNext = $derived(this.index < this.queue.length - 1);
	hasPrevious = $derived(this.index > 0);

	#host: MediaHost | null = null;
	#volumeStorage: VolumeStorage | null | undefined;
	#pendingSeek: number | null = null;
	#source: { url: string; mimeType: string } | null = null;
	/**
	 * True from a track change until its source is set. The element still
	 * holds the previous track then, and its events must not be taken for
	 * the new one.
	 */
	#awaitingSource = false;
	#autoplay = $state(false);
	#mediaPaused = $state(true);
	/** True until the restored position is applied, so position 0 is never reported. */
	#seeking = $state(false);
	#sleepTimer: ReturnType<typeof setTimeout> | null = null;
	#stallTimer: ReturnType<typeof setTimeout> | null = null;
	/** Waiting for data since the last `playing`, on the current source. */
	#stalled = false;
	#subtitleFor: number | null = null;
	readonly #sessionControls = {
		play: () => void this.play(),
		pause: () => this.pause(),
		previous: () => this.previous(),
		next: () => this.next(),
		back: () => this.back(),
		forward: () => this.forward(),
		seekBy: (offset: number) => this.seekBy(offset),
		seekTo: (seconds: number) => this.seekTo(seconds)
	};

	constructor(options: PlayerOptions = {}) {
		this.#volumeStorage = options.volumeStorage;
		this.volumeAdjustable =
			options.volumeAdjustable ??
			(typeof document === 'undefined' || canSetVolume(() => document.createElement('audio')));
		const saved = readVolume(this.#volumeStorage);
		this.muted = saved.muted;
		// A volume saved by another browser engine means nothing where the
		// hardware buttons own it, and would only be written to a no-op.
		this.volume = this.volumeAdjustable ? saved.volume : 1;
	}

	readonly reporter = new ProgressReporter({
		put: putProgress,
		finish: finishWork,
		clear: clearProgress,
		now: () => Date.now(),
		onError: (error) => console.warn('progress report failed', error),
		onSaved: (_workId, kind) => {
			this.lastSavedKind = kind;
			this.savedRevision++;
		}
	});

	readonly #loader = new TrackLoader({
		prepare: prepareTrack,
		getTask,
		sleep,
		canPlay: () => detectCanPlay(browserCanPlayType),
		mediaToken: async () => {
			try {
				return await ensureMediaToken();
			} catch {
				return session.mediaToken;
			}
		},
		refreshMediaToken,
		probeStatus: async (url) => {
			try {
				return (await fetch(url, { method: 'HEAD' })).status;
			} catch {
				return 0;
			}
		},
		probePlaylist: async (url) => {
			try {
				const response = await fetch(url, { cache: 'no-store' });
				return { status: response.status, set: response.ok ? playlistSet(await response.text()) : null };
			} catch {
				return { status: 0, set: null };
			}
		},
		setSource: (url, mimeType, positionMs) => {
			// A seek the user made while the track was still preparing wins
			// over the position the load started with.
			this.#pendingSeek ??= positionMs / 1000;
			this.#seeking = true;
			// Kept so a host that attaches after the load finished still gets it.
			this.#source = { url, mimeType };
			this.#awaitingSource = false;
			this.#clearStall();
			this.#host?.setSource(url, mimeType);
			// Every source, a recovered or fallback one included, starts at the
			// saved volume rather than the element's default.
			this.#host?.setVolume(this.volume, this.muted);
		},
		onState: (state) => {
			this.load = state;
		},
		now: () => Date.now()
	});

	get active(): boolean {
		return this.work !== null && this.current !== null;
	}

	// While a source loads, controls toggle the pending intent, not the paused element.
	get paused(): boolean {
		return this.#seeking && (this.load.state !== 'error' || this.load.retrying)
			? !this.#autoplay
			: this.#mediaPaused;
	}

	/** Connects the media element. Returns the function that disconnects it. */
	attach(host: MediaHost): () => void {
		this.#host = host;
		host.setRate(this.rate);
		host.setVolume(this.volume, this.muted);
		installHandlers(this.#sessionControls);
		// Playback may have started before the player chunk loaded, and
		// Vidstack clears the lock-screen metadata when it connects.
		this.#updateMetadata();
		// The Vidstack chunk can arrive after the first prepare finished.
		if (this.#source) {
			host.setSource(this.#source.url, this.#source.mimeType);
			host.setVolume(this.volume, this.muted);
		}
		if (typeof document === 'undefined') {
			return () => {
				if (this.#host === host) this.#host = null;
			};
		}
		const onHidden = () => {
			if (document.visibilityState === 'hidden') this.#flush({ beforeUnload: true });
		};
		// A loader waiting for the server tries again at once when the network
		// or the page comes back, instead of finishing its backoff first. A
		// phone may come back online with its screen still off.
		const onOnline = () => this.#loader.retryNow();
		const onVisible = () => {
			if (document.visibilityState === 'visible') this.#loader.retryNow();
		};
		document.addEventListener('visibilitychange', onHidden);
		window.addEventListener('pagehide', onHidden);
		document.addEventListener('visibilitychange', onVisible);
		window.addEventListener('online', onOnline);
		return () => {
			document.removeEventListener('visibilitychange', onHidden);
			window.removeEventListener('pagehide', onHidden);
			document.removeEventListener('visibilitychange', onVisible);
			window.removeEventListener('online', onOnline);
			if (this.#host === host) this.#host = null;
		};
	}

	/** Starts a work at a track. The queue is that track's directory group. */
	playWork(work: WorkDetail, start: StartPoint): void {
		const queue = buildQueue(work.tracks, start.groupPath);
		const index = queue.findIndex((t) => t.id === start.trackId);
		if (index === -1) return;
		this.#flush();
		this.work = {
			id: work.id,
			code: work.code,
			title: work.title,
			circle: work.circle,
			has_cover: work.has_cover,
			cover_version: work.cover_version
		};
		this.queue = queue;
		this.groupPath = start.groupPath;
		this.reporter.begin(work.id);
		this.#startTrack(index, start.positionMs);
	}

	playTrack(work: WorkDetail, track: TrackItem): void {
		if (!isPlayable(track)) return;
		this.playWork(work, {
			groupPath: groupOf(track.path),
			trackId: track.id,
			positionMs: 0,
			resumed: false
		});
	}

	isCurrentWork(workId: number): boolean {
		return this.work?.id === workId;
	}

	async play(): Promise<void> {
		if (!this.active) return;
		this.#autoplay = true;
		if (this.load.state === 'error' && !this.load.retrying && this.current) {
			this.#startTrack(this.index, this.currentTime * 1000);
			return;
		}
		// Until the new source is in place the host still holds the previous
		// track. Autoplay starts the right one once it can play.
		if (this.#seeking || this.load.state !== 'ready') return;
		await this.#host?.play().catch(() => {});
	}

	pause(): void {
		this.#autoplay = false;
		this.#host?.pause();
	}

	toggle(): void {
		if (this.paused) void this.play();
		else this.pause();
	}

	next(): void {
		if (!this.hasNext) return;
		this.#flush();
		this.#autoplay = true;
		this.#startTrack(this.index + 1, 0);
	}

	previous(): void {
		if (this.currentTime > RESTART_THRESHOLD_SECONDS || !this.hasPrevious) {
			this.seekTo(0);
			return;
		}
		this.#flush();
		this.#autoplay = true;
		this.#startTrack(this.index - 1, 0);
	}

	jumpTo(index: number): void {
		if (index < 0 || index >= this.queue.length || index === this.index) return;
		this.#flush();
		this.#autoplay = true;
		this.#startTrack(index, 0);
	}

	seekBy(offset: number): void {
		this.seekTo(this.currentTime + offset);
	}

	back(): void {
		this.seekBy(-this.seekBackSeconds);
	}

	forward(): void {
		this.seekBy(this.seekForwardSeconds);
	}

	/** Takes the steps from the settings, keeping the defaults for missing ones. */
	setSeekSteps(back: unknown, forward: unknown): void {
		this.seekBackSeconds = seekSeconds(back, DEFAULT_SEEK_BACK_SECONDS);
		this.seekForwardSeconds = seekSeconds(forward, DEFAULT_SEEK_FORWARD_SECONDS);
	}

	seekTo(seconds: number): void {
		const limit = this.duration > 0 ? this.duration : Infinity;
		const target = Math.min(Math.max(0, seconds), limit);
		if (this.#seeking) {
			this.#pendingSeek = target;
		} else {
			this.#host?.seek(target);
		}
		this.currentTime = target;
	}

	setRate(rate: number): void {
		this.rate = rate;
		this.#host?.setRate(rate);
	}

	/** Sets the volume from 0 to 1. Turning it up while muted unmutes, as on iOS. */
	setVolume(volume: number): void {
		if (!this.volumeAdjustable) return;
		this.volume = clampVolume(volume);
		if (this.volume > 0) this.muted = false;
		this.#applyVolume();
	}

	/** Moves the volume by `delta`, for the arrow keys. */
	changeVolume(delta: number): void {
		// Turning a muted player down would only lose the volume it returns to.
		if (this.muted && delta < 0) return;
		this.setVolume(this.volume + delta);
	}

	toggleMute(): void {
		if (this.muted || (this.volumeAdjustable && this.volume === 0)) {
			this.muted = false;
			if (this.volumeAdjustable && this.volume === 0) this.volume = UNMUTE_VOLUME;
		} else {
			this.muted = true;
		}
		this.#applyVolume();
	}

	/** Takes a change the user made with the browser's own controls. */
	adoptVolume(volume: number, muted: boolean): void {
		if (this.volumeAdjustable) this.volume = clampVolume(volume);
		this.muted = muted;
		writeVolume({ volume: this.volume, muted: this.muted }, this.#volumeStorage);
	}

	#applyVolume(): void {
		this.#host?.setVolume(this.volume, this.muted);
		writeVolume({ volume: this.volume, muted: this.muted }, this.#volumeStorage);
	}

	setSleep(mode: SleepMode): void {
		if (this.#sleepTimer) clearTimeout(this.#sleepTimer);
		this.#sleepTimer = null;
		this.sleepMode = mode;
		this.sleepEndsAt = null;
		if (typeof mode === 'number') {
			const ms = mode * 60_000;
			this.sleepEndsAt = Date.now() + ms;
			this.#sleepTimer = setTimeout(() => {
				this.pause();
				this.sleepMode = 'off';
				this.sleepEndsAt = null;
				this.#sleepTimer = null;
			}, ms);
		}
	}

	/** The account changed: stop without reporting anything for the old one. */
	endSession(): void {
		this.reporter.reset();
		this.stop({ report: false });
	}

	/** Stops playback and forgets the queue, for sign-out and cleared progress. */
	stop({ report = true }: { report?: boolean } = {}): void {
		if (report) this.#flush();
		this.work = null;
		this.#loader.cancel();
		this.#clearStall();
		this.#autoplay = false;
		this.#host?.pause();
		this.#host?.clearSource();
		this.setSleep('off');
		this.queue = [];
		this.index = 0;
		this.currentTime = 0;
		this.duration = 0;
		this.#mediaPaused = true;
		this.subtitle = { state: 'none' };
		this.#subtitleFor = null;
		setMetadata(null);
	}

	/** Clears the saved progress. Reports not yet sent for the work are dropped. */
	async clearProgress(workId: number): Promise<void> {
		if (this.isCurrentWork(workId)) this.stop();
		await this.reporter.clear(workId);
	}

	/** 标记为听过 for the current track, or the given one when the work is not playing. */
	async markFinished(workId: number, fallback: { trackId: number; groupPath: string }) {
		const loaded = this.isCurrentWork(workId) && this.current;
		const trackId = loaded ? this.current!.id : fallback.trackId;
		const groupPath = loaded ? this.groupPath : fallback.groupPath;
		if (loaded) this.#flush();
		// Unless it keeps playing, a later report would count as a replay.
		const stillPlaying = loaded && !this.#mediaPaused;
		await this.reporter.markFinished(workId, trackId, groupPath, { seal: !stillPlaying });
	}

	// Events from the media host.

	onCanPlay(): void {
		if (this.#awaitingSource) return;
		// Vidstack's media session may connect only now; reapply ours over it.
		this.#updateMetadata();
		if (this.#pendingSeek !== null) {
			const target = this.#pendingSeek;
			this.#pendingSeek = null;
			if (target > 0) this.#host?.seek(target);
			this.currentTime = target;
		}
		this.#seeking = false;
		if (this.#autoplay) void this.#host?.play().catch(() => {});
	}

	onPlay(): void {
		this.#mediaPaused = false;
		this.#autoplay = true;
		if (this.work) this.reporter.begin(this.work.id);
	}

	onPause(): void {
		this.#mediaPaused = true;
		// A pause that did not come through pause(), from iOS fullscreen
		// controls, unplugged headphones or a call, is the user's too, so a
		// recovery reload must not resume. Pauses while a track or a
		// recovered source loads are the player's own.
		if (!this.#awaitingSource && !this.#seeking) this.#autoplay = false;
		this.#clearStall();
		this.#flush();
	}

	/**
	 * The element ran out of data. For HLS the playlist is probed when this
	 * lasts, and again every interval while it does, because a segment
	 * rejected for an expired token may never surface as an error.
	 */
	onWaiting(): void {
		if (this.#awaitingSource || !this.#source || !isHlsType(this.#source.mimeType)) return;
		this.#clearStall();
		this.#stalled = true;
		const source = this.#source;
		this.#stallTimer = setTimeout(() => {
			this.#stallTimer = null;
			const recovering = () => {
				// As in onError: seeks during the reload, which can include a
				// new prepare, are kept for the new source.
				this.#seeking = true;
				this.#pendingSeek ??= this.currentTime;
			};
			void this.#loader.handleStall(Math.round(this.currentTime * 1000), recovering).then(() => {
				// Same source and nothing played since: keep watching.
				if (this.#stalled && this.#source === source) this.onWaiting();
			});
		}, STALL_PROBE_MS);
	}

	/** Data arrived again after waiting. */
	onPlaying(): void {
		this.#clearStall();
	}

	#clearStall(): void {
		if (this.#stallTimer) clearTimeout(this.#stallTimer);
		this.#stallTimer = null;
		this.#stalled = false;
	}

	onTimeUpdate(seconds: number): void {
		if (this.#seeking || this.#awaitingSource) return;
		this.currentTime = seconds;
		setPositionState(this.duration, seconds, this.rate);
		const snapshot = this.#snapshot();
		if (snapshot && !this.#mediaPaused) this.reporter.tick(snapshot);
	}

	onDurationChange(seconds: number): void {
		// The old element keeps loading while the next track prepares, and its
		// late duration would clamp every seek in the new track.
		if (this.#awaitingSource) return;
		// A source change reports 0 before the new file's metadata arrives. The
		// track's known duration stays until the real one replaces it.
		if (Number.isFinite(seconds) && seconds > 0) this.duration = seconds;
	}

	onEnded(): void {
		if (this.#awaitingSource) return;
		const snapshot = this.#snapshot(this.duration || this.currentTime);
		const isLast = !this.hasNext;
		if (snapshot) this.reporter.trackEnded(snapshot, isLast);
		if (this.sleepMode === 'end-of-track') {
			this.setSleep('off');
			this.#autoplay = false;
			return;
		}
		if (!isLast) {
			this.#autoplay = true;
			this.#startTrack(this.index + 1, 0);
		} else {
			this.#autoplay = false;
		}
	}

	onError(): void {
		if (this.#awaitingSource) return;
		// Until the recovered source is in place, seeks are kept for it rather
		// than sent to the element that just failed.
		this.#seeking = true;
		this.#pendingSeek ??= this.currentTime;
		void this.#loader.handleMediaError(Math.round(this.currentTime * 1000));
	}

	#snapshot(seconds = this.currentTime): ProgressSnapshot | null {
		const { work, current } = this;
		if (!work || !current) return null;
		return {
			workId: work.id,
			trackId: current.id,
			positionMs: Math.round(seconds * 1000),
			groupPath: this.groupPath
		};
	}

	#flush({ beforeUnload = false } = {}): void {
		if (this.#seeking) return;
		const snapshot = this.#snapshot();
		if (!snapshot) return;
		if (beforeUnload) this.reporter.flushBeforeUnload(snapshot);
		else this.reporter.flush(snapshot);
	}

	#startTrack(index: number, positionMs: number): void {
		const track = this.queue[index];
		if (!track || !this.work) return;
		this.index = index;
		this.currentTime = positionMs / 1000;
		this.duration = (track.duration_ms ?? 0) / 1000;
		this.#autoplay = true;
		this.#seeking = true;
		this.#pendingSeek = positionMs / 1000;
		this.#source = null;
		this.#awaitingSource = true;
		this.#clearStall();
		this.#host?.pause();
		this.#updateMetadata();
		this.#loadSubtitle(track);
		void this.#loader.load(track.id, positionMs);
	}

	#updateMetadata(): void {
		const { work } = this;
		if (!work) return;
		// Vidstack registers its own handlers when the element connects, which
		// can happen after attach(), so ours are set again for every track.
		installHandlers(this.#sessionControls);
		setMetadata({
			title: this.currentTitle,
			workTitle: work.title,
			circle: work.circle?.name ?? null,
			artworkUrl:
				work.has_cover && typeof location !== 'undefined'
					? new URL(coverUrl(work, 'thumb', session.mediaToken), location.href).href
					: null
		});
	}

	#loadSubtitle(track: TrackItem): void {
		this.#subtitleFor = track.id;
		if (!track.has_subtitle) {
			this.subtitle = { state: 'none' };
			return;
		}
		this.subtitle = { state: 'loading' };
		getSubtitle(track.id).then(
			(text) => {
				if (this.#subtitleFor === track.id) this.subtitle = { state: 'ready', cues: parseVtt(text) };
			},
			() => {
				if (this.#subtitleFor === track.id) this.subtitle = { state: 'error' };
			}
		);
	}
}

export const player = new Player();
