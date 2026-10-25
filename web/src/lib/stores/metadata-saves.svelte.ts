import { SvelteMap, SvelteSet } from 'svelte/reactivity';

/**
 * Works with a metadata save in flight. It lives outside the editor, so
 * leaving the page and coming back cannot start a second PUT for the same
 * work. Two PUTs in flight could reach the server out of order and keep the
 * older edit.
 */
class MetadataSaves {
	#pending = new SvelteSet<number>();
	#saved = new SvelteMap<number, number>();
	// Refresh tasks per work, kept until they end, so leaving the detail page
	// and coming back still blocks editing and follows the same task.
	#refreshTasks = new SvelteMap<number, number>();
	#refreshStarting = new SvelteSet<number>();

	/** A refresh of the work is starting or running, so the editor waits for it. */
	isRefreshing(workId: number): boolean {
		return this.#refreshTasks.has(workId) || this.#refreshStarting.has(workId);
	}

	refreshTask(workId: number): number | null {
		return this.#refreshTasks.get(workId) ?? null;
	}

	/** Covers the time between sending the refresh and learning its task ID. */
	markRefreshStarting(workId: number, starting: boolean): void {
		if (starting) this.#refreshStarting.add(workId);
		else this.#refreshStarting.delete(workId);
	}

	refreshStarted(workId: number, taskId: number): void {
		this.#refreshTasks.set(workId, taskId);
	}

	/** Forgets the task once it ended or cannot be followed. A newer task stays. */
	refreshEnded(workId: number, taskId: number): void {
		if (this.#refreshTasks.get(workId) === taskId) this.#refreshTasks.delete(workId);
	}

	/**
	 * Successful saves of a work so far. A detail page watches it and reloads
	 * when it grows, even when the save was sent from an earlier visit.
	 */
	savedCount(workId: number): number {
		return this.#saved.get(workId) ?? 0;
	}

	isSaving(workId: number): boolean {
		return this.#pending.has(workId);
	}

	/** Runs the save, or returns null without calling it while one for the work is in flight. */
	async run<T>(workId: number, save: () => Promise<T>): Promise<T | null> {
		if (this.#pending.has(workId)) return null;
		this.#pending.add(workId);
		try {
			const result = await save();
			this.#saved.set(workId, this.savedCount(workId) + 1);
			return result;
		} finally {
			this.#pending.delete(workId);
		}
	}
}

export const metadataSaves = new MetadataSaves();
