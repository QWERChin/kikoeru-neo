import { debounce } from './debounce';

/** Navigates for a typed query and returns the target URL, or null when nothing changed. */
export type CommitQuery = (value: string) => string | null;

// A few targets are enough: only navigations started while one is still
// loading are outstanding at the same time.
const MAX_OUTSTANDING = 5;

/**
 * The timing rules of the search box, kept out of the component so they can
 * be tested: a debounced commit, nothing while an IME composes a word, and
 * a pending commit dropped when the user navigates elsewhere first.
 */
export class SearchInputController {
	#commit: CommitQuery;
	#debounced;
	#composing = false;
	// URLs this box navigated to. Seeing one start or land is our own echo,
	// which must neither cancel newer typing nor overwrite it.
	#ownTargets: string[] = [];
	#ownQueries: string[] = [];

	constructor(commit: CommitQuery, delayMs: number) {
		this.#commit = commit;
		this.#debounced = debounce((value: string) => this.#run(value), delayMs);
	}

	get pending(): boolean {
		return this.#debounced.pending;
	}

	#run(value: string): void {
		const target = this.#commit(value);
		if (target === null) return;
		this.#ownTargets = [...this.#ownTargets.slice(1 - MAX_OUTSTANDING), target];
		this.#ownQueries = [...this.#ownQueries.slice(1 - MAX_OUTSTANDING), value];
	}

	input(value: string, isComposing = false): void {
		if (isComposing || this.#composing) return;
		this.#debounced(value);
	}

	/** An earlier word may still be scheduled, and it must not land mid-composition. */
	compositionStart(): void {
		this.#composing = true;
		this.#debounced.cancel();
	}

	compositionEnd(value: string): void {
		this.#composing = false;
		this.#debounced(value);
	}

	submit(value: string): void {
		this.#debounced.cancel();
		this.#run(value);
	}

	/**
	 * Called before every navigation with its target. One this box did not
	 * start, such as a link or back, supersedes whatever is still typed.
	 */
	navigationStarting(target: string | null): void {
		const own = target === null ? -1 : this.#ownTargets.indexOf(target);
		if (own >= 0) {
			this.#ownTargets = this.#ownTargets.slice(own + 1);
			return;
		}
		// The outside navigation also supersedes ours that have not landed, so
		// their queries are no longer echoes to ignore.
		this.#debounced.cancel();
		this.#ownTargets = [];
		this.#ownQueries = [];
	}

	/**
	 * The query of the URL that is now shown. Returns the text the box should
	 * show, or null to leave what the user is typing alone.
	 */
	urlChanged(q: string): string | null {
		const own = this.#ownQueries.indexOf(q);
		if (own >= 0) {
			this.#ownQueries = this.#ownQueries.slice(own + 1);
			return null;
		}
		if (this.#debounced.pending || this.#composing) return null;
		return q;
	}

	dispose(): void {
		this.#debounced.cancel();
	}
}
