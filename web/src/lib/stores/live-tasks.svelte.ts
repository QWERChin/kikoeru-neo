import { ApiError } from '../api/errors';
import type { TaskSnapshot } from '../api/types';
import { isTerminal, type TaskWatch, type TaskWatchOptions, type WatchMode } from '../tasks/task-watch';

export interface LiveTask {
	id: number;
	/** What the page calls the task, such as the library a scan belongs to. */
	label: string;
	snapshot: TaskSnapshot | null;
	mode: WatchMode;
	/** Why the task cannot be followed, or why a cancel or a move failed. */
	error: string | null;
	cancelling: boolean;
	/** Waiting for the server to move this queued scan to the front. */
	moving: boolean;
}

/**
 * Streams one page keeps open at most. Browsers allow six HTTP/1.1
 * connections per origin, so more tasks than this are polled instead and
 * ordinary requests such as a cancel still get through.
 */
export const MAX_LIVE_STREAMS = 3;

export interface LiveTaskDeps {
	watch(taskId: number, options: TaskWatchOptions): TaskWatch;
	cancel(taskId: number): Promise<void>;
	/** Only the scan list offers this, so other pages leave it out. */
	moveToFront?(taskId: number): Promise<void>;
}

/**
 * The tasks a page follows live. Each page that shows progress owns one and
 * calls `closeAll` when it goes away: that drops the streams and leaves the
 * tasks running. Only `cancel` asks the server to stop a task.
 */
export class LiveTasks {
	items = $state<LiveTask[]>([]);
	#watches = new Map<number, TaskWatch>();
	#streaming = new Set<number>();
	#deps: LiveTaskDeps;
	#onFinished: (snapshot: TaskSnapshot) => void;
	// Set once the owning page is gone. A request that answers after that
	// must not open a stream nobody will close.
	#closed = false;

	constructor(deps: LiveTaskDeps, onFinished: (snapshot: TaskSnapshot) => void = () => {}) {
		this.#deps = deps;
		this.#onFinished = onFinished;
	}

	get(id: number): LiveTask | undefined {
		return this.items.find((t) => t.id === id);
	}

	/** Starts following a task. Following it again only updates the label. */
	follow(id: number, label: string, snapshot: TaskSnapshot | null = null): void {
		if (this.#closed) return;
		const existing = this.get(id);
		if (existing) {
			existing.label = label;
			if (this.#watches.has(id) || (existing.snapshot && isTerminal(existing.snapshot.state))) return;
		} else {
			this.items.unshift({ id, label, snapshot, mode: 'connecting', error: null, cancelling: false, moving: false });
		}
		const update = (change: Partial<LiveTask>) => {
			const item = this.get(id);
			if (item) Object.assign(item, change);
		};
		const pollOnly = this.#streaming.size >= MAX_LIVE_STREAMS;
		if (!pollOnly) this.#streaming.add(id);
		const watch = this.#deps.watch(id, {
			pollOnly,
			onSnapshot: (s) => {
				update({ snapshot: s });
				if (isTerminal(s.state)) update({ cancelling: false });
			},
			onMode: (mode) => update({ mode }),
			onError: (e) => update({ error: e.message })
		});
		this.#watches.set(id, watch);
		void watch.finished.then((last) => {
			if (this.#watches.get(id) === watch) {
				this.#watches.delete(id);
				this.#streaming.delete(id);
			}
			if (last) this.#onFinished(last);
		});
	}

	async cancel(id: number): Promise<void> {
		const item = this.get(id);
		if (!item || item.cancelling) return;
		item.cancelling = true;
		item.error = null;
		try {
			await this.#deps.cancel(id);
			// The watch stays open: the stream delivers the cancelled snapshot.
		} catch (e) {
			const current = this.get(id);
			if (!current) return;
			current.cancelling = false;
			current.error = e instanceof ApiError ? e.message : '取消失败';
		}
	}

	/**
	 * Moves a queued scan to the front of the scan queue. The new positions
	 * of this scan and the ones it passed arrive through their watches.
	 */
	async moveToFront(id: number): Promise<void> {
		const item = this.get(id);
		const move = this.#deps.moveToFront;
		if (!item || item.moving || !move) return;
		item.moving = true;
		item.error = null;
		try {
			await move(id);
		} catch (e) {
			const current = this.get(id);
			if (current) current.error = e instanceof ApiError ? e.message : '没能移到最前';
		} finally {
			const current = this.get(id);
			if (current) current.moving = false;
		}
	}

	/** Removes a finished task from the list. */
	dismiss(id: number): void {
		this.#watches.get(id)?.close();
		this.#watches.delete(id);
		this.#streaming.delete(id);
		this.items = this.items.filter((t) => t.id !== id);
	}

	/** Closes every stream for good, without touching the tasks themselves. */
	closeAll(): void {
		this.#closed = true;
		for (const watch of this.#watches.values()) watch.close();
		this.#watches.clear();
		this.#streaming.clear();
	}
}
