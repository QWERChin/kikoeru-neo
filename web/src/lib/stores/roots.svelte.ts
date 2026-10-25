import { ApiError, listRoots } from '../api/index';
import type { RootStatus } from '../api/types';
import { session } from './session.svelte';

interface Loaded {
	key: string;
	roots: RootStatus[];
}

/**
 * The media library list from /api/admin/roots, shared by the sidebar, the
 * search filters and the settings page. Only admins may read it, so for
 * everyone else it stays empty and no request is sent. Answers are tied to
 * the session that asked, so another account never sees them.
 */
class RootsStore {
	#loaded = $state<Loaded | null>(null);
	#failed = $state<{ key: string; error: ApiError } | null>(null);
	#inFlight: { key: string; promise: Promise<void> } | null = null;
	// Bumped by `reload`, so an answer requested before a change is dropped.
	#generation = 0;

	get visible(): boolean {
		return session.user?.is_admin === true;
	}

	get list(): RootStatus[] | null {
		return this.#loaded?.key === session.key ? this.#loaded.roots : null;
	}

	get error(): ApiError | null {
		return this.#failed?.key === session.key ? this.#failed.error : null;
	}

	/** The last refresh failed, so `list` shows the status from before it. */
	get stale(): boolean {
		return this.error !== null && this.list !== null;
	}

	/** Loads once per session unless `force` asks for fresh status. */
	load(force = false): Promise<void> {
		const key = session.key;
		if (!this.visible || key === null) return Promise.resolve();
		if (this.#inFlight?.key === key) return this.#inFlight.promise;
		if (!force && this.list !== null) return Promise.resolve();
		const generation = this.#generation;
		const promise = listRoots()
			.then((roots) => {
				// A late answer for a previous account must not replace the current one's.
				if (session.key !== key || generation !== this.#generation) return;
				this.#loaded = { key, roots };
				this.#failed = null;
			})
			.catch((e: unknown) => {
				if (session.key !== key || generation !== this.#generation) return;
				const error = e instanceof ApiError ? e : new ApiError(0, 'unknown', '加载失败');
				this.#failed = { key, error };
			})
			.finally(() => {
				if (this.#inFlight?.promise === promise) this.#inFlight = null;
			});
		this.#inFlight = { key, promise };
		return promise;
	}

	/**
	 * Fresh status after the page added, changed or deleted a root. A
	 * request already on its way may have been answered before the change,
	 * so it is dropped instead of joined.
	 */
	reload(): Promise<void> {
		this.#generation++;
		this.#inFlight = null;
		return this.load(true);
	}

	nameOf(id: string | null): string | null {
		if (id === null) return null;
		return this.list?.find((r) => r.id === id)?.display_name ?? null;
	}
}

export const roots = new RootsStore();
