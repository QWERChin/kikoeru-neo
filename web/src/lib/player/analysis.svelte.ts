import { SvelteMap } from 'svelte/reactivity';
import type { AnalysisPending } from '../api/index';
import type { Loudness, TaskSnapshot, TrackItem } from '../api/types';
import { POLL_INTERVAL_MS, abortError } from './prepare';
import { decodeWaveform } from './waveform';

/**
 * How many 202 answers one track may get before it is given up. The task
 * finishing and the next request still answering 202 means the waveform
 * was evicted in between, which should not repeat.
 */
export const MAX_ANALYSIS_ROUNDS = 3;

const FAILED_STATES = new Set<TaskSnapshot['state']>(['failed', 'cancelled', 'interrupted']);

export interface AnalysisDeps {
	getTask(taskId: number): Promise<TaskSnapshot>;
	/** Resolves after `ms`, or rejects with an AbortError when `signal` aborts. */
	sleep(ms: number, signal: AbortSignal): Promise<void>;
}

function isPending(answer: unknown): answer is AnalysisPending {
	return (
		typeof answer === 'object' &&
		answer !== null &&
		!(answer instanceof Uint8Array) &&
		(answer as { state?: unknown }).state === 'processing'
	);
}

/**
 * Asks for a result of the track's analysis. A 202 answer is followed by
 * polling its task, then the same request again. Any error, a failed task
 * or too many rounds give null: loudness and the waveform are extras, so
 * playback goes on without them. Aborting `signal` (the track changed)
 * also gives null and stops all polling.
 */
export async function untilAnalysed<T>(
	request: () => Promise<T | AnalysisPending>,
	signal: AbortSignal,
	deps: AnalysisDeps
): Promise<T | null> {
	try {
		for (let round = 0; round < MAX_ANALYSIS_ROUNDS; round++) {
			const answer = await request();
			if (signal.aborted) throw abortError();
			if (!isPending(answer)) return answer;
			for (;;) {
				await deps.sleep(POLL_INTERVAL_MS, signal);
				const task = await deps.getTask(answer.task_id);
				if (signal.aborted) throw abortError();
				if (task.state === 'done') break;
				if (FAILED_STATES.has(task.state)) return null;
			}
		}
		return null;
	} catch {
		return null;
	}
}

export interface TrackAnalysisDeps extends AnalysisDeps {
	getLoudness(trackId: number): Promise<Loudness | AnalysisPending>;
	getWaveform(trackId: number): Promise<Uint8Array | AnalysisPending>;
}

export interface FollowOptions {
	/** The desktop layout shows the waveform. Phones never fetch it. */
	waveform: boolean;
	/** Normalisation is on, so an unmeasured track's loudness is worth asking for. */
	loudness: boolean;
	/**
	 * `prepare` has answered for this track. Before that the server does not
	 * know whether a transcode will fetch a remote file, and an analysis
	 * request could make it download the file a second time.
	 */
	prepared: boolean;
}

/** Only audio gets loudness and a waveform. Videos are left at their own level. */
function analysable(track: TrackItem | null): track is TrackItem {
	return track !== null && track.kind === 'audio' && !track.missing;
}

/**
 * Loudness and the waveform of the playing track. Each is requested when a
 * track starts and only while it is wanted, and a track change stops the
 * previous track's requests and polling.
 */
export class TrackAnalysis {
	/** The current track's peaks, 0 to 1, once they arrived. */
	waveform = $state<{ trackId: number; peaks: Float32Array } | null>(null);

	/**
	 * Reuse measurements only for the same track object from a loaded detail.
	 * A fresh detail can invalidate loudness without changing the file size.
	 */
	readonly #measured = new SvelteMap<number, { track: TrackItem; size: number | null; loudness: Loudness }>();
	#wave: { trackId: number; abort: AbortController } | null = null;
	#loud: { track: TrackItem; size: number | null; abort: AbortController } | null = null;
	readonly #deps: TrackAnalysisDeps;

	constructor(deps: TrackAnalysisDeps) {
		this.#deps = deps;
	}

	/** The loudness the gain uses for `track`, null while it is unknown. */
	loudnessOf(track: TrackItem | null): Loudness | null {
		if (!analysable(track)) return null;
		if (track.loudness) return track.loudness;
		const measured = this.#measured.get(track.id);
		return measured && measured.track === track && measured.size === track.size ? measured.loudness : null;
	}

	/** Starts, keeps or stops the requests for the track that is playing now. */
	follow(track: TrackItem | null, options: FollowOptions): void {
		this.#followWaveform(analysable(track) && options.waveform ? track : null, options.prepared);
		const needsLoudness = analysable(track) && options.loudness && this.loudnessOf(track) === null;
		this.#followLoudness(needsLoudness ? track : null, options.prepared);
	}

	/** Forgets everything, for another account. */
	reset(): void {
		this.follow(null, { waveform: false, loudness: false, prepared: false });
		this.#measured.clear();
	}

	#followWaveform(track: TrackItem | null, canStart: boolean): void {
		if (this.#wave && this.#wave.trackId === track?.id) return;
		this.#wave?.abort.abort();
		this.#wave = null;
		if (this.waveform?.trackId !== track?.id) this.waveform = null;
		if (!track || this.waveform || !canStart) return;
		const abort = new AbortController();
		this.#wave = { trackId: track.id, abort };
		void untilAnalysed(() => this.#deps.getWaveform(track.id), abort.signal, this.#deps).then((body) => {
			if (abort.signal.aborted) return;
			const peaks = body ? decodeWaveform(body) : null;
			if (peaks) this.waveform = { trackId: track.id, peaks };
		});
	}

	#followLoudness(track: TrackItem | null, canStart: boolean): void {
		if (this.#loud && this.#loud.track === track && this.#loud.size === track.size) return;
		this.#loud?.abort.abort();
		this.#loud = null;
		if (!track || !canStart) return;
		const abort = new AbortController();
		this.#loud = { track, size: track.size, abort };
		void untilAnalysed(() => this.#deps.getLoudness(track.id), abort.signal, this.#deps).then((value) => {
			if (abort.signal.aborted) return;
			if (value) this.#measured.set(track.id, { track, size: track.size, loudness: value });
		});
	}
}
