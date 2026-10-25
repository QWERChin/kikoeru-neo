import { SvelteMap, SvelteSet } from 'svelte/reactivity';
import { addFavorite, removeFavorite } from '../api/index';
import { ApiError } from '../api/errors';
import type { WorkSummary } from '../api/types';
import { session } from './session.svelte';

export interface FavoriteDeps {
	add(workId: number): Promise<void>;
	remove(workId: number): Promise<void>;
	/** Identifies the signed-in session, so one account never sees another's toggles. */
	sessionKey(): string | null;
}

export interface FavoriteFailure {
	workId: number;
	/** What the user tried to do, so the message can say so. */
	wanted: boolean;
	message: string;
}

type FavoriteWork = Pick<WorkSummary, 'id' | 'favorite'>;

/**
 * Favorite state that a toggle changes before the server answers. Grids and
 * the detail page fetch their own copies of a work, so the override lives
 * here and every copy reads through it. A failed request puts the previous
 * value back. While a request runs the work's button is disabled, which
 * keeps the rollback target unambiguous.
 */
export class FavoriteStore {
	#deps: FavoriteDeps;
	// Keyed by session and work, so signing in as someone else starts clean
	// without having to clear anything during rendering.
	#overrides = new SvelteMap<string, boolean>();
	#pending = new SvelteSet<string>();
	// Counts settled requests, so a page can tell whether its response was
	// requested after a toggle finished and therefore already includes it.
	#settledCount = 0;
	#settledAt = new Map<string, number>();
	failure = $state<FavoriteFailure | null>(null);

	constructor(deps: FavoriteDeps) {
		this.#deps = deps;
	}

	#key(workId: number, sessionKey = this.#deps.sessionKey()): string {
		return `${sessionKey ?? ''}:${workId}`;
	}

	isFavorite(work: FavoriteWork): boolean {
		return this.#overrides.get(this.#key(work.id)) ?? work.favorite;
	}

	isPending(workId: number): boolean {
		return this.#pending.has(this.#key(workId));
	}

	/** Resolves to whether the server accepted the change. */
	async toggle(work: FavoriteWork): Promise<boolean> {
		const sessionKey = this.#deps.sessionKey();
		const key = this.#key(work.id, sessionKey);
		if (this.#pending.has(key)) return false;
		const before = this.isFavorite(work);
		const wanted = !before;
		this.#overrides.set(key, wanted);
		this.#pending.add(key);
		this.failure = null;
		try {
			await (wanted ? this.#deps.add(work.id) : this.#deps.remove(work.id));
			return true;
		} catch (error) {
			this.#overrides.set(key, before);
			// A 401 already signed the user out, so a message would only flash.
			if (this.#deps.sessionKey() === sessionKey) {
				const message = error instanceof ApiError ? error.message : '请稍后再试';
				this.failure = { workId: work.id, wanted, message };
			}
			return false;
		} finally {
			this.#pending.delete(key);
			this.#settledCount += 1;
			this.#settledAt.set(key, this.#settledCount);
		}
	}

	/** Taken right before a page requests works, for `reconcile`. */
	mark(): number {
		return this.#settledCount;
	}

	/**
	 * Drops overrides that a fresh response supersedes. A response requested
	 * after a toggle settled is the server's truth, including changes made
	 * on another device. One requested earlier may predate the toggle, so
	 * the override stays, and so does one whose request is still running.
	 */
	reconcile(works: readonly FavoriteWork[], mark: number): void {
		for (const work of works) {
			const key = this.#key(work.id);
			const settledAt = this.#settledAt.get(key);
			if (this.#pending.has(key) || settledAt === undefined || settledAt > mark) continue;
			this.#overrides.delete(key);
			this.#settledAt.delete(key);
		}
	}

	dismissFailure(): void {
		this.failure = null;
	}
}

export const favorites = new FavoriteStore({
	add: addFavorite,
	remove: removeFavorite,
	sessionKey: () => session.key
});
