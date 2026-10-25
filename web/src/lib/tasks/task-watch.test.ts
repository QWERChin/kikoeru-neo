import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import running from '../../../../contract/m2/task-running.json';
import { ApiError } from '../api/errors';
import type { TaskSnapshot } from '../api/types';
import {
	POLL_INTERVAL_MS,
	RECONNECT_DELAYS_MS,
	STREAM_FAILURES_BEFORE_POLLING,
	STREAM_IDLE_TIMEOUT_MS,
	watchTask,
	type TaskWatchDeps,
	type WatchMode
} from './task-watch';

const snap = (state: TaskSnapshot['state'], progress: number | null = null): TaskSnapshot => ({
	...(running as TaskSnapshot),
	id: 91,
	kind: 'scan',
	state,
	progress
});

const event = (s: TaskSnapshot) => `event: snapshot\ndata: ${JSON.stringify(s)}\n\n`;

function controlledStream() {
	const encoder = new TextEncoder();
	let controller!: ReadableStreamDefaultController<Uint8Array>;
	let cancelled = false;
	const stream = new ReadableStream<Uint8Array>({
		start(c) {
			controller = c;
		},
		cancel() {
			cancelled = true;
		}
	});
	return {
		stream,
		send: (text: string) => controller.enqueue(encoder.encode(text)),
		end: () => controller.close(),
		get cancelled() {
			return cancelled;
		}
	};
}

function recorder() {
	const snapshots: TaskSnapshot[] = [];
	const modes: WatchMode[] = [];
	const errors: ApiError[] = [];
	return {
		snapshots,
		modes,
		errors,
		options: {
			onSnapshot: (s: TaskSnapshot) => snapshots.push(s),
			onMode: (m: WatchMode) => modes.push(m),
			onError: (e: ApiError) => errors.push(e)
		}
	};
}

const networkError = () => new ApiError(0, 'network_error', '无法连接到服务器');

describe('watchTask', () => {
	beforeEach(() => vi.useFakeTimers());
	afterEach(() => vi.useRealTimers());

	it('delivers the first snapshot, then every change, and ends on a terminal one', async () => {
		const s = controlledStream();
		const deps: TaskWatchDeps = { open: vi.fn(async () => s.stream), poll: vi.fn() };
		const r = recorder();
		const watch = watchTask(91, r.options, deps);
		await vi.advanceTimersByTimeAsync(0);
		s.send(event(snap('running', 0.1)) + ': ping\n\n');
		s.send(event(snap('running', 0.5)));
		s.send(event(snap('done', 1)));
		await expect(watch.finished).resolves.toMatchObject({ state: 'done' });
		expect(r.snapshots.map((x) => x.progress)).toEqual([0.1, 0.5, 1]);
		expect(r.modes).toEqual(['connecting', 'stream', 'closed']);
		expect(deps.open).toHaveBeenCalledTimes(1);
		expect(deps.poll).not.toHaveBeenCalled();
	});

	it('reconnects after the stream drops and reads the fresh snapshot first', async () => {
		const first = controlledStream();
		const second = controlledStream();
		const open = vi.fn().mockResolvedValueOnce(first.stream).mockResolvedValueOnce(second.stream);
		const r = recorder();
		const watch = watchTask(91, r.options, { open, poll: vi.fn() });
		await vi.advanceTimersByTimeAsync(0);
		first.send(event(snap('running', 0.2)));
		first.end();
		await vi.advanceTimersByTimeAsync(0);
		expect(r.modes.at(-1)).toBe('reconnecting');
		expect(open).toHaveBeenCalledTimes(1);
		// A stream that delivered events resets the backoff to its first step.
		await vi.advanceTimersByTimeAsync(RECONNECT_DELAYS_MS[0]);
		expect(open).toHaveBeenCalledTimes(2);
		second.send(event(snap('running', 0.6)));
		second.send(event(snap('cancelled', 0.6)));
		await expect(watch.finished).resolves.toMatchObject({ state: 'cancelled' });
		expect(r.snapshots.map((x) => x.state)).toEqual(['running', 'running', 'cancelled']);
	});

	it('backs off between failed connections and then polls the snapshot', async () => {
		const open = vi.fn(async () => {
			throw networkError();
		});
		const poll = vi
			.fn()
			.mockResolvedValueOnce(snap('running', 0.4))
			.mockResolvedValueOnce(snap('done', 1));
		const r = recorder();
		const watch = watchTask(91, r.options, { open, poll });
		await vi.advanceTimersByTimeAsync(0);
		expect(open).toHaveBeenCalledTimes(1);
		await vi.advanceTimersByTimeAsync(RECONNECT_DELAYS_MS[1] - 1);
		expect(open).toHaveBeenCalledTimes(1);
		await vi.advanceTimersByTimeAsync(1);
		expect(open).toHaveBeenCalledTimes(2);
		await vi.advanceTimersByTimeAsync(RECONNECT_DELAYS_MS[2]);
		expect(open).toHaveBeenCalledTimes(STREAM_FAILURES_BEFORE_POLLING);
		expect(r.modes.at(-1)).toBe('polling');
		expect(poll).toHaveBeenCalledTimes(1);
		await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS);
		await expect(watch.finished).resolves.toMatchObject({ state: 'done' });
		expect(r.snapshots.map((x) => x.state)).toEqual(['running', 'done']);
		expect(open).toHaveBeenCalledTimes(STREAM_FAILURES_BEFORE_POLLING);
	});

	it('polls at once when the answer is not an event stream', async () => {
		const open = vi.fn(async () => {
			throw new ApiError(200, 'bad_response', '服务器没有返回事件流');
		});
		const poll = vi.fn(async () => snap('done', 1));
		const r = recorder();
		await expect(watchTask(91, r.options, { open, poll }).finished).resolves.toMatchObject({ state: 'done' });
		expect(open).toHaveBeenCalledTimes(1);
		expect(r.modes).toEqual(['connecting', 'polling', 'closed']);
	});

	it('reopens a stream that stays silent past the idle timeout', async () => {
		const silent = controlledStream();
		const next = controlledStream();
		const open = vi.fn().mockResolvedValueOnce(silent.stream).mockResolvedValueOnce(next.stream);
		const watch = watchTask(91, recorder().options, { open, poll: vi.fn() });
		await vi.advanceTimersByTimeAsync(STREAM_IDLE_TIMEOUT_MS);
		expect(silent.cancelled).toBe(true);
		await vi.advanceTimersByTimeAsync(RECONNECT_DELAYS_MS[1]);
		expect(open).toHaveBeenCalledTimes(2);
		next.send(event(snap('done', 1)));
		await expect(watch.finished).resolves.toMatchObject({ state: 'done' });
	});

	it('stops on a 404 without retrying', async () => {
		const open = vi.fn(async () => {
			throw new ApiError(404, 'not_found', '没有这个任务');
		});
		const r = recorder();
		await expect(watchTask(91, r.options, { open, poll: vi.fn() }).finished).resolves.toBeNull();
		expect(r.errors.map((e) => e.code)).toEqual(['not_found']);
		expect(open).toHaveBeenCalledTimes(1);
	});

	it('close only drops the connection: no more snapshots and no retry', async () => {
		const s = controlledStream();
		const open = vi.fn(async () => s.stream);
		const r = recorder();
		const watch = watchTask(91, r.options, { open, poll: vi.fn() });
		await vi.advanceTimersByTimeAsync(0);
		s.send(event(snap('running', 0.3)));
		await vi.advanceTimersByTimeAsync(0);
		watch.close();
		await expect(watch.finished).resolves.toBeNull();
		expect(s.cancelled).toBe(true);
		await vi.advanceTimersByTimeAsync(60_000);
		expect(open).toHaveBeenCalledTimes(1);
		expect(r.snapshots).toHaveLength(1);
		expect(r.errors).toEqual([]);
		expect(r.modes.at(-1)).toBe('closed');
	});

	it('polls without opening a stream when asked to', async () => {
		const open = vi.fn();
		const poll = vi.fn(async () => snap('done', 1));
		const r = recorder();
		await expect(watchTask(91, { ...r.options, pollOnly: true }, { open, poll }).finished).resolves.toMatchObject({
			state: 'done'
		});
		expect(open).not.toHaveBeenCalled();
		expect(r.modes).toEqual(['polling', 'closed']);
	});
});
