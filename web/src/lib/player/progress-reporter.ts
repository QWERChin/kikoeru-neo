import type { FinishRequest, ProgressRequest } from '../api/types';

/** book/src/api.md: report every 15 seconds while playing. */
export const REPORT_INTERVAL_MS = 15_000;

export interface ProgressSnapshot {
	workId: number;
	trackId: number;
	positionMs: number;
	groupPath: string;
}

export interface ReporterDeps {
	put(workId: number, body: ProgressRequest, options: { keepalive: boolean }): Promise<void>;
	finish(workId: number, body: FinishRequest): Promise<void>;
	clear(workId: number): Promise<void>;
	now(): number;
	onError?(error: unknown): void;
	/** A report, finish or clear for the work reached the server. */
	onSaved?(workId: number, kind: 'put' | 'finish' | 'clear'): void;
}

interface Settle {
	resolve(): void;
	reject(error: unknown): void;
}

type Op = (
	| { kind: 'put'; workId: number; body: ProgressRequest; keepalive: boolean }
	| { kind: 'finish'; workId: number; body: FinishRequest }
	| { kind: 'clear'; workId: number }
) & { settle?: Settle };

/**
 * Ordinary reports run serially. Hidden and closing pages also send past
 * the queue; the server rejects stale reports using their session sequence.
 * A newer report replaces the pending tail for the same work.
 *
 * Finishing or clearing a work seals it: later reports for it are dropped
 * until playback of that work starts again (`begin`). Otherwise the pause
 * that follows the last track, or the stop after a clear, would write the
 * progress back and the server would count it as a replay.
 */
export class ProgressReporter {
	#deps: ReporterDeps;
	#queue: Op[] = [];
	#running: Promise<void> | null = null;
	#lastQueuedAt = -Infinity;
	#sessionId: string | null = null;
	#sequence = 0;
	#sealed = new Set<number>();
	/** The unload report sent past the queue, while it is in flight. */
	#sideWrite: Promise<void> | null = null;

	constructor(deps: ReporterDeps) {
		this.#deps = deps;
	}

	/** Playback of a work started, so its progress is reported again. */
	begin(workId: number): void {
		this.#sealed.delete(workId);
	}

	/** Called on every time update while playing; reports at most every 15 seconds. */
	tick(snapshot: ProgressSnapshot): void {
		if (this.#sealed.has(snapshot.workId)) return;
		if (this.#deps.now() - this.#lastQueuedAt < REPORT_INTERVAL_MS) return;
		this.#enqueuePut(snapshot.workId, this.#body(snapshot), false);
	}

	/** Reports right away: on pause and track change. */
	flush(snapshot: ProgressSnapshot): void {
		if (this.#sealed.has(snapshot.workId)) return;
		this.#enqueuePut(snapshot.workId, this.#body(snapshot), false);
	}

	/**
	 * The page is being hidden or unloaded and may never run another task.
	 * The report goes out with keepalive at once, even past a request in
	 * flight. Its queued copy reuses the identity so it can retry and notify
	 * onSaved if the page survives, without overwriting newer reports.
	 */
	flushBeforeUnload(snapshot: ProgressSnapshot): void {
		if (this.#sealed.has(snapshot.workId)) return;
		const body = this.#body(snapshot);
		if (this.#running) {
			// Requests queued after this one wait for it, so a clear or finish
			// can never land before it.
			const write = this.#deps
				.put(snapshot.workId, body, { keepalive: true })
				.catch((error: unknown) => this.#deps.onError?.(error));
			const pending: Promise<void> = Promise.all([this.#sideWrite, write]).then(() => {
				if (this.#sideWrite === pending) this.#sideWrite = null;
			});
			this.#sideWrite = pending;
		}
		this.#enqueuePut(snapshot.workId, body, true);
	}

	/**
	 * A track played to its end. Only the last track of the group finishes
	 * the work. Any other track just reports its final position.
	 */
	trackEnded(snapshot: ProgressSnapshot, isLastInGroup: boolean): void {
		this.flush(snapshot);
		if (!isLastInGroup || this.#sealed.has(snapshot.workId)) return;
		this.#sealed.add(snapshot.workId);
		this.#enqueue({
			kind: 'finish',
			workId: snapshot.workId,
			body: { track_id: snapshot.trackId, group_path: snapshot.groupPath, manual: false }
		});
	}

	/**
	 * 标记为听过: the server skips the last-track check for a manual finish.
	 * `seal` is for a work that is not playing on, see the class comment.
	 */
	markFinished(
		workId: number,
		trackId: number,
		groupPath: string,
		{ seal = false }: { seal?: boolean } = {}
	): Promise<void> {
		if (seal) this.#sealed.add(workId);
		return this.#enqueueSettled({
			kind: 'finish',
			workId,
			body: { track_id: trackId, group_path: groupPath, manual: true }
		});
	}

	/**
	 * Drops every report for the work that has not been sent yet, then sends
	 * the clear after any request already in flight.
	 */
	clear(workId: number): Promise<void> {
		this.#sealed.add(workId);
		this.#queue = this.#queue.filter((op) => op.kind === 'clear' || op.workId !== workId);
		return this.#enqueueSettled({ kind: 'clear', workId });
	}

	/** Drops everything not yet sent, for a change of account. */
	reset(): void {
		const dropped = this.#queue;
		this.#queue = [];
		this.#sealed.clear();
		this.#lastQueuedAt = -Infinity;
		this.#sessionId = null;
		this.#sequence = 0;
		for (const op of dropped) op.settle?.reject(new Error('signed out'));
	}

	/** Resolves once every queued request has been sent. */
	async idle(): Promise<void> {
		while (this.#running || this.#sideWrite) await (this.#running ?? this.#sideWrite);
	}

	#body(snapshot: ProgressSnapshot): ProgressRequest {
		// getRandomValues also works on the supported plain-HTTP origins.
		this.#sessionId ??= Array.from(crypto.getRandomValues(new Uint8Array(16)), (byte) =>
			byte.toString(16).padStart(2, '0')
		).join('');
		return {
			track_id: snapshot.trackId,
			position_ms: Math.max(0, Math.round(snapshot.positionMs)),
			group_path: snapshot.groupPath,
			report_order: { session_id: this.#sessionId, sequence: ++this.#sequence }
		};
	}

	#enqueuePut(workId: number, body: ProgressRequest, keepalive: boolean): void {
		this.#lastQueuedAt = this.#deps.now();
		const waiting = this.#queue.find(
			(op): op is Extract<Op, { kind: 'put' }> =>
				op.kind === 'put' && op.workId === workId
		);
		// Replace the whole body, including its identity: an unload bypass may
		// still be using the previous body.
		if (waiting && this.#queue.indexOf(waiting) === this.#queue.length - 1) {
			waiting.body = body;
			waiting.keepalive ||= keepalive;
			return;
		}
		this.#enqueue({ kind: 'put', workId, body, keepalive });
	}

	/** Queues a request whose caller wants to know how it went. */
	#enqueueSettled(op: Op): Promise<void> {
		return new Promise((resolve, reject) => {
			this.#enqueue({ ...op, settle: { resolve, reject } });
		});
	}

	#enqueue(op: Op): void {
		this.#queue.push(op);
		this.#running ??= this.#drain().finally(() => {
			this.#running = null;
		});
	}

	async #drain(): Promise<void> {
		while (this.#queue.length > 0) {
			// Wait with the next request still queued, so a reset() for a new
			// account during the wait can still drop it.
			if (this.#sideWrite) {
				await this.#sideWrite;
				continue;
			}
			const op = this.#queue.shift();
			if (!op) break;
			try {
				await this.#send(op);
				this.#deps.onSaved?.(op.workId, op.kind);
				op.settle?.resolve();
			} catch (error) {
				if (op.settle) op.settle.reject(error);
				else this.#deps.onError?.(error);
			}
		}
	}

	#send(op: Op): Promise<void> {
		switch (op.kind) {
			case 'put':
				return this.#deps.put(op.workId, op.body, {
					keepalive: op.keepalive
				});
			case 'finish':
				return this.#deps.finish(op.workId, op.body);
			case 'clear':
				return this.#deps.clear(op.workId);
		}
	}
}
