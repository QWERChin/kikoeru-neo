import type { TasksQuery } from '../api/query';
import type { Page, TaskSnapshot, TaskState } from '../api/types';

/** Largest page `GET /api/tasks` returns; more active probes than this show as "at least". */
export const ACTIVITY_PAGE_SIZE = 100;
export const ACTIVITY_POLL_MS = 3_000;
export const IDLE_POLL_MS = 15_000;

export interface ProbeCounts {
	queued: number;
	running: number;
	/** A page was full, so the real numbers may be higher. */
	capped: boolean;
}

/**
 * Counts the probes among the queued and running tasks. The task list has
 * no kind filter, so this reads both states and counts by kind.
 */
export function countProbes(queued: Page<TaskSnapshot>, running: Page<TaskSnapshot>): ProbeCounts {
	const probes = (page: Page<TaskSnapshot>) => page.items.filter((t) => t.kind === 'probe').length;
	return {
		queued: probes(queued),
		running: probes(running),
		capped: queued.total > queued.items.length || running.total > running.items.length
	};
}

export interface TaskActivityDeps {
	listTasks(query: TasksQuery): Promise<Page<TaskSnapshot>>;
}

/**
 * What is going on in the background while the settings page is open: scans
 * the page should follow, and the probes that scans leave behind. Scans and
 * probes are separate tasks, so a finished scan never hides running probes.
 */
export class TaskActivity {
	probes = $state<ProbeCounts | null>(null);
	/** Queued and running scans, newest first. */
	scans = $state<TaskSnapshot[]>([]);
	#deps: TaskActivityDeps;
	#timer: ReturnType<typeof setTimeout> | undefined;
	#stopped = true;
	// A newer loop replaces an older one that is still waiting for its answer.
	#generation = 0;
	#requests = 0;

	constructor(deps: TaskActivityDeps) {
		this.#deps = deps;
	}

	get busy(): boolean {
		return this.scans.length > 0 || (this.probes !== null && this.probes.queued + this.probes.running > 0);
	}

	async refresh(): Promise<void> {
		// Polls can overlap after a poke, so only the newest answer is shown.
		const request = ++this.#requests;
		try {
			const [queued, running, queuedScans, runningScans] = await Promise.all([
				this.#deps.listTasks({ state: 'queued', page_size: ACTIVITY_PAGE_SIZE }),
				this.#deps.listTasks({ state: 'running', page_size: ACTIVITY_PAGE_SIZE }),
				this.#scans('queued'),
				this.#scans('running')
			]);
			if (request !== this.#requests) return;
			this.probes = countProbes(queued, running);
			this.scans = [...runningScans, ...queuedScans]
				.sort((a, b) => b.created_at - a.created_at || b.id - a.id);
		} catch {
			// Keeps the last counts. The task list below shows its own errors.
		}
	}

	async #scans(state: TaskState): Promise<TaskSnapshot[]> {
		// Scanning can queue thousands of probes. Exclude them from discovery,
		// but read every remaining page so other task kinds cannot hide scans.
		const scans: TaskSnapshot[] = [];
		for (let page = 1; ; page++) {
			const result = await this.#deps.listTasks({ state, exclude_kind: 'probe', page, page_size: ACTIVITY_PAGE_SIZE });
			scans.push(...result.items.filter((t) => t.kind === 'scan'));
			if (page * ACTIVITY_PAGE_SIZE >= result.total) return scans;
		}
	}

	/** Refreshes now and then keeps polling, faster while something is active. */
	start(): void {
		this.#stopped = false;
		void this.#loop();
	}

	stop(): void {
		this.#stopped = true;
		clearTimeout(this.#timer);
	}

	async #loop(): Promise<void> {
		const generation = ++this.#generation;
		clearTimeout(this.#timer);
		await this.refresh();
		if (this.#stopped || generation !== this.#generation) return;
		this.#timer = setTimeout(() => void this.#loop(), this.busy ? ACTIVITY_POLL_MS : IDLE_POLL_MS);
	}

	/** Polls again soon, for example right after a scan was started. */
	poke(): void {
		if (!this.#stopped) void this.#loop();
	}
}
