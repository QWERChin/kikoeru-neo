import { isScanRunning } from './errors';

export const SCAN_RETRY_MS = 3_000;

export interface RetryOptions {
	signal: AbortSignal;
	intervalMs?: number;
	/** Injected by tests. */
	sleep?: (ms: number, signal: AbortSignal) => Promise<void>;
}

function abortableSleep(ms: number, signal: AbortSignal): Promise<void> {
	return new Promise((resolve) => {
		const timer = setTimeout(done, ms);
		signal.addEventListener('abort', done, { once: true });
		function done() {
			clearTimeout(timer);
			signal.removeEventListener('abort', done);
			resolve();
		}
	});
}

/**
 * Repeats `run` while it fails with 409 `scan_running`. Task snapshots do
 * not say which library a scan belongs to, so asking the server again is
 * the only way to learn that the library's scan ended. Resolves `false`
 * when the signal stops the wait, and rethrows any other error.
 */
export async function retryWhileScanning(run: () => Promise<void>, options: RetryOptions): Promise<boolean> {
	const { signal, intervalMs = SCAN_RETRY_MS, sleep = abortableSleep } = options;
	for (;;) {
		if (signal.aborted) return false;
		try {
			await run();
			return true;
		} catch (e) {
			if (!isScanRunning(e)) throw e;
		}
		await sleep(intervalMs, signal);
	}
}
