export interface Debounced<A extends unknown[]> {
	(...args: A): void;
	/** Runs a pending call now, for example when the form is submitted. */
	flush(): void;
	/** Drops a pending call, for example when the page is left. */
	cancel(): void;
	readonly pending: boolean;
}

/**
 * Calls `fn` once input has been quiet for `ms`, with the latest arguments.
 * The search box uses it so typing does not send a request per key.
 */
export function debounce<A extends unknown[]>(fn: (...args: A) => void, ms: number): Debounced<A> {
	let timer: ReturnType<typeof setTimeout> | null = null;
	let lastArgs: A | null = null;

	const run = () => {
		timer = null;
		const args = lastArgs;
		lastArgs = null;
		if (args) fn(...args);
	};

	const debounced = ((...args: A) => {
		lastArgs = args;
		if (timer !== null) clearTimeout(timer);
		timer = setTimeout(run, ms);
	}) as Debounced<A>;

	debounced.flush = () => {
		if (timer === null) return;
		clearTimeout(timer);
		run();
	};
	debounced.cancel = () => {
		if (timer !== null) clearTimeout(timer);
		timer = null;
		lastArgs = null;
	};
	Object.defineProperty(debounced, 'pending', { get: () => timer !== null });
	return debounced;
}
