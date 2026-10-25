import { describe, expect, it, vi } from 'vitest';
import running from '../../../../contract/m2/task-running.json';
import { ApiError } from '../api/errors';
import type { TaskSnapshot } from '../api/types';
import type { TaskWatchOptions } from '../tasks/task-watch';
import { LiveTasks, MAX_LIVE_STREAMS } from './live-tasks.svelte';

const snap = (state: TaskSnapshot['state']): TaskSnapshot => ({ ...(running as TaskSnapshot), id: 91, kind: 'scan', state });

function fakeWatch() {
	let options!: TaskWatchOptions;
	let finish!: (s: TaskSnapshot | null) => void;
	const close = vi.fn(() => finish(null));
	const watch = vi.fn((_id: number, o: TaskWatchOptions) => {
		options = o;
		return { close, finished: new Promise<TaskSnapshot | null>((r) => (finish = r)) };
	});
	return {
		watch,
		close,
		emit: (s: TaskSnapshot) => options.onSnapshot(s),
		end: (s: TaskSnapshot) => {
			options.onSnapshot(s);
			finish(s);
		}
	};
}

describe('LiveTasks', () => {
	it('cancel asks the server and keeps the stream open for the cancelled snapshot', async () => {
		const w = fakeWatch();
		const cancel = vi.fn(async () => {});
		const finished = vi.fn();
		const live = new LiveTasks({ watch: w.watch, cancel }, finished);
		live.follow(91, '本地作品');
		w.emit(snap('running'));
		await live.cancel(91);
		expect(cancel).toHaveBeenCalledWith(91);
		expect(w.close).not.toHaveBeenCalled();
		expect(live.get(91)?.cancelling).toBe(true);
		w.end(snap('cancelled'));
		await Promise.resolve();
		expect(live.get(91)?.snapshot?.state).toBe('cancelled');
		expect(live.get(91)?.cancelling).toBe(false);
		expect(finished).toHaveBeenCalledWith(expect.objectContaining({ state: 'cancelled' }));
	});

	it('closing the page only closes the streams and never cancels', () => {
		const w = fakeWatch();
		const cancel = vi.fn(async () => {});
		const live = new LiveTasks({ watch: w.watch, cancel });
		live.follow(91, '本地作品');
		live.closeAll();
		expect(w.close).toHaveBeenCalledTimes(1);
		expect(cancel).not.toHaveBeenCalled();
	});

	it('shows why a cancel failed and lets the user try again', async () => {
		const w = fakeWatch();
		const cancel = vi.fn(async () => {
			throw new ApiError(400, 'invalid_request', '任务已经结束');
		});
		const live = new LiveTasks({ watch: w.watch, cancel });
		live.follow(91, '本地作品');
		await live.cancel(91);
		expect(live.get(91)).toMatchObject({ cancelling: false, error: '任务已经结束' });
	});

	it('moves a queued scan to the front and leaves the new position to the watch', async () => {
		const w = fakeWatch();
		let answer!: () => void;
		const moveToFront = vi.fn(() => new Promise<void>((resolve) => (answer = resolve)));
		const live = new LiveTasks({ watch: w.watch, cancel: vi.fn(), moveToFront });
		live.follow(91, '本地作品');
		w.emit({ ...snap('queued'), queue_position: 2 });
		const moving = live.moveToFront(91);
		expect(live.get(91)?.moving).toBe(true);
		void live.moveToFront(91);
		expect(moveToFront).toHaveBeenCalledTimes(1);
		answer();
		await moving;
		expect(moveToFront).toHaveBeenCalledWith(91);
		expect(live.get(91)).toMatchObject({ moving: false, error: null });
		expect(live.get(91)?.snapshot?.queue_position).toBe(2);
		w.emit({ ...snap('queued'), queue_position: 1 });
		expect(live.get(91)?.snapshot?.queue_position).toBe(1);
	});

	it('shows why a move failed', async () => {
		const w = fakeWatch();
		const moveToFront = vi.fn(async () => {
			throw new ApiError(400, 'invalid_request', '扫描已经开始或已经结束');
		});
		const live = new LiveTasks({ watch: w.watch, cancel: vi.fn(), moveToFront });
		live.follow(91, '本地作品');
		await live.moveToFront(91);
		expect(live.get(91)).toMatchObject({ moving: false, error: '扫描已经开始或已经结束' });
	});

	it('following the same task twice keeps one stream', () => {
		const w = fakeWatch();
		const live = new LiveTasks({ watch: w.watch, cancel: vi.fn() });
		live.follow(91, '扫描任务 #91');
		live.follow(91, '本地作品');
		expect(w.watch).toHaveBeenCalledTimes(1);
		expect(live.items).toHaveLength(1);
		expect(live.get(91)?.label).toBe('本地作品');
	});

	it('opens no stream for a task that answers after the page closed', () => {
		const w = fakeWatch();
		const live = new LiveTasks({ watch: w.watch, cancel: vi.fn() });
		live.closeAll();
		live.follow(91, '本地作品');
		expect(w.watch).not.toHaveBeenCalled();
		expect(live.items).toHaveLength(0);
	});

	it('polls the tasks beyond the stream limit so requests keep a connection', () => {
		const w = fakeWatch();
		const live = new LiveTasks({ watch: w.watch, cancel: vi.fn() });
		for (let id = 1; id <= MAX_LIVE_STREAMS + 1; id++) live.follow(id, `#${id}`);
		const pollOnly = w.watch.mock.calls.map(([, options]) => options.pollOnly);
		expect(pollOnly).toEqual([...Array<boolean>(MAX_LIVE_STREAMS).fill(false), true]);
	});
});
