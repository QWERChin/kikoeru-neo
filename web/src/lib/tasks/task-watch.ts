// Follows one task through GET /api/tasks/{id}/events, the way
// book/src/contract.md describes the stream: every connection starts with a
// snapshot, the last snapshot is terminal, and closing the stream never
// cancels the task. When the stream keeps failing or a proxy does not pass
// it through, the watcher polls GET /api/tasks/{id} instead.
import { ApiError, CLIENT_ERROR_CODES } from '../api/errors';
import { SseParser } from '../api/sse';
import type { TaskSnapshot, TaskState } from '../api/types';

/** Delays before reconnecting, indexed by the number of failed attempts in a row. */
export const RECONNECT_DELAYS_MS = [1_000, 2_000, 4_000, 8_000, 15_000] as const;
/** Connections in a row that end without one event before the watcher polls instead. */
export const STREAM_FAILURES_BEFORE_POLLING = 3;
export const POLL_INTERVAL_MS = 2_000;
export const MAX_POLL_INTERVAL_MS = 30_000;
/**
 * The server sends `: ping` every 15 seconds. A stream that stays silent for
 * three of those is held back by something in between, so it is reopened.
 */
export const STREAM_IDLE_TIMEOUT_MS = 45_000;

const TERMINAL: readonly TaskState[] = ['done', 'failed', 'cancelled', 'interrupted'];

export function isTerminal(state: TaskState): boolean {
	return TERMINAL.includes(state);
}

export type WatchMode = 'connecting' | 'stream' | 'reconnecting' | 'polling' | 'closed';

export interface TaskWatchDeps {
	open(taskId: number, signal: AbortSignal): Promise<ReadableStream<Uint8Array>>;
	poll(taskId: number): Promise<TaskSnapshot>;
}

export interface TaskWatchOptions {
	onSnapshot(snapshot: TaskSnapshot): void;
	onMode?(mode: WatchMode): void;
	/** The task cannot be followed at all, for example it does not exist. The watch ends. */
	onError?(error: ApiError): void;
	/** Poll from the start without opening a stream, to save a connection. */
	pollOnly?: boolean;
}

export interface TaskWatch {
	/** Stops following the task. The task itself keeps running. */
	close(): void;
	/** The terminal snapshot, or null when the watch was closed or failed first. */
	finished: Promise<TaskSnapshot | null>;
}

// Errors that another attempt cannot fix: the task is gone, the session
// expired or the user is not an admin.
function isFatal(error: unknown): error is ApiError {
	return (
		error instanceof ApiError &&
		error.status >= 400 &&
		error.status < 500 &&
		error.status !== 408 &&
		error.status !== 429
	);
}

function sleep(ms: number, signal: AbortSignal): Promise<void> {
	return new Promise((resolve) => {
		if (signal.aborted) return resolve();
		const timer = setTimeout(done, ms);
		function done() {
			clearTimeout(timer);
			signal.removeEventListener('abort', done);
			resolve();
		}
		signal.addEventListener('abort', done);
	});
}

/** Reads `snapshot` events until the stream ends, calling `onSnapshot` for each. */
async function readSnapshots(
	body: ReadableStream<Uint8Array>,
	signal: AbortSignal,
	onSnapshot: (snapshot: TaskSnapshot) => boolean
): Promise<void> {
	const reader = body.getReader();
	const decoder = new TextDecoder();
	const parser = new SseParser();
	const stop = () => void reader.cancel().catch(() => {});
	signal.addEventListener('abort', stop);
	try {
		for (;;) {
			const { done, value } = await reader.read();
			if (done || signal.aborted) return;
			for (const event of parser.push(decoder.decode(value, { stream: true }))) {
				if (event.event !== 'snapshot') continue;
				let snapshot: TaskSnapshot;
				try {
					snapshot = JSON.parse(event.data) as TaskSnapshot;
				} catch {
					throw new ApiError(200, CLIENT_ERROR_CODES.badResponse, '任务事件不是合法的 JSON');
				}
				if (onSnapshot(snapshot)) return;
			}
		}
	} finally {
		signal.removeEventListener('abort', stop);
		stop();
	}
}

export function watchTask(taskId: number, options: TaskWatchOptions, deps: TaskWatchDeps): TaskWatch {
	const controller = new AbortController();
	const { signal } = controller;
	let mode: WatchMode | null = null;
	let last: TaskSnapshot | null = null;

	const setMode = (next: WatchMode) => {
		if (mode === next) return;
		mode = next;
		options.onMode?.(next);
	};

	// True when the snapshot ends the task, so the caller stops reading.
	const deliver = (snapshot: TaskSnapshot): boolean => {
		if (signal.aborted) return true;
		last = snapshot;
		options.onSnapshot(snapshot);
		return isTerminal(snapshot.state);
	};

	const fail = (error: ApiError): null => {
		if (!signal.aborted) options.onError?.(error);
		return null;
	};

	async function streamOnce(): Promise<{ events: boolean; ended: boolean }> {
		// Each attempt has its own controller, so the idle timer can drop one
		// silent connection without closing the whole watch.
		const attempt = new AbortController();
		const forward = () => attempt.abort();
		signal.addEventListener('abort', forward);
		let idle: ReturnType<typeof setTimeout> | undefined;
		const touch = () => {
			clearTimeout(idle);
			idle = setTimeout(() => attempt.abort(), STREAM_IDLE_TIMEOUT_MS);
		};
		let events = false;
		let ended = false;
		try {
			touch();
			const body = await deps.open(taskId, attempt.signal);
			setMode('stream');
			touch();
			await readSnapshots(watchBytes(body, touch), attempt.signal, (snapshot) => {
				events = true;
				ended = deliver(snapshot);
				return ended;
			});
		} catch (e) {
			// An idle timeout aborts only the attempt and counts as a failure.
			if (signal.aborted || attempt.signal.aborted) return { events, ended };
			throw e;
		} finally {
			clearTimeout(idle);
			signal.removeEventListener('abort', forward);
		}
		return { events, ended };
	}

	async function pollUntilDone(): Promise<TaskSnapshot | null> {
		setMode('polling');
		let delay = POLL_INTERVAL_MS;
		while (!signal.aborted) {
			try {
				const snapshot = await deps.poll(taskId);
				if (deliver(snapshot)) return signal.aborted ? null : snapshot;
				delay = POLL_INTERVAL_MS;
			} catch (e) {
				if (signal.aborted) return null;
				if (isFatal(e)) return fail(e);
				delay = Math.min(delay * 2, MAX_POLL_INTERVAL_MS);
			}
			await sleep(delay, signal);
		}
		return null;
	}

	async function run(): Promise<TaskSnapshot | null> {
		if (options.pollOnly) return pollUntilDone();
		setMode('connecting');
		let failures = 0;
		while (!signal.aborted) {
			try {
				const { events, ended } = await streamOnce();
				if (ended) return signal.aborted ? null : last;
				// A stream that delivered something worked, so the next attempt
				// starts over with the shortest delay.
				failures = events ? 0 : failures + 1;
			} catch (e) {
				if (signal.aborted) return null;
				if (isFatal(e)) return fail(e);
				// Not an event stream at all: retrying will not change that.
				if (e instanceof ApiError && e.code === CLIENT_ERROR_CODES.badResponse) return pollUntilDone();
				failures += 1;
			}
			if (signal.aborted) return null;
			if (failures >= STREAM_FAILURES_BEFORE_POLLING) return pollUntilDone();
			setMode('reconnecting');
			await sleep(RECONNECT_DELAYS_MS[Math.min(failures, RECONNECT_DELAYS_MS.length - 1)] ?? POLL_INTERVAL_MS, signal);
		}
		return null;
	}

	const finished = run()
		.catch((e: unknown) => fail(e instanceof ApiError ? e : new ApiError(0, CLIENT_ERROR_CODES.unknown, '任务进度读取失败')))
		.finally(() => setMode('closed'));

	return {
		close: () => controller.abort(),
		finished
	};
}

/** Passes the body through and calls `onChunk` for every chunk, pings included. */
function watchBytes(body: ReadableStream<Uint8Array>, onChunk: () => void): ReadableStream<Uint8Array> {
	return body.pipeThrough(
		new TransformStream<Uint8Array, Uint8Array>({
			transform(chunk, controller) {
				onChunk();
				controller.enqueue(chunk);
			}
		})
	);
}
