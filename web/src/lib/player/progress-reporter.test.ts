import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProgressRequest } from '../api/types';
import { REPORT_INTERVAL_MS, ProgressReporter, type ProgressSnapshot } from './progress-reporter';

interface Deferred {
	resolve: () => void;
	reject: (error: unknown) => void;
}

/** Records every request and lets the test decide when each one finishes. */
function setup() {
	let now = 0;
	const calls: string[] = [];
	const pending = new Map<number, Deferred>();
	const reports: { workId: number; body: ProgressRequest; keepalive: boolean }[] = [];
	const saved: string[] = [];
	const respond = (label: string) =>
		new Promise<void>((resolve, reject) => {
			pending.set(calls.length, { resolve, reject });
			calls.push(label);
		});
	const reporter = new ProgressReporter({
		put: (workId, body, options) => {
			reports.push({ workId, body, keepalive: options.keepalive });
			return respond(
				`put ${workId} ${body.track_id} ${body.position_ms}${options.keepalive ? ' keepalive' : ''}`
			);
		},
		finish: (workId, body) =>
			respond(`finish ${workId} ${body.track_id} ${body.group_path}${body.manual ? ' manual' : ''}`),
		clear: (workId) => respond(`clear ${workId}`),
		now: () => now,
		onError: vi.fn(),
		onSaved: (workId, kind) => saved.push(`${kind} ${workId}`)
	});
	async function completeRequest(index: number, error?: unknown) {
		const next = pending.get(index);
		if (!next) throw new Error(`request ${index} is not in flight`);
		pending.delete(index);
		if (error) next.reject(error);
		else next.resolve();
		await new Promise((r) => setTimeout(r, 0));
	}
	return {
		reporter,
		calls,
		reports,
		saved,
		advance: (ms: number) => (now += ms),
		completeRequest,
		/** Finishes the oldest request in flight and lets the queue move on. */
		async complete(error?: unknown) {
			const index = pending.keys().next().value;
			if (index === undefined) throw new Error('no request in flight');
			await completeRequest(index, error);
		}
	};
}

const at = (positionMs: number, trackId = 101): ProgressSnapshot => ({
	workId: 12,
	trackId,
	positionMs,
	groupPath: 'mp3/SE有'
});

describe('ProgressReporter', () => {
	afterEach(() => vi.unstubAllGlobals());

	it('orders an immediate unload report and its queued copy without waiting for the old report', async () => {
		const { reporter, reports, saved, completeRequest } = setup();
		reporter.flushBeforeUnload(at(10_000));
		reporter.flushBeforeUnload(at(20_000));
		expect(reports).toHaveLength(2);
		const [a, b] = reports;
		expect(a!.body.report_order).toEqual({
			session_id: expect.stringMatching(/^[0-9a-f]{32}$/), sequence: 1
		});
		expect(b).toEqual({
			workId: 12,
			keepalive: true,
			body: {
				track_id: 101, position_ms: 20_000, group_path: 'mp3/SE有',
				report_order: { session_id: a!.body.report_order!.session_id, sequence: 2 }
			}
		});
		await completeRequest(1);
		expect(saved).toEqual([]);
		expect(reports).toHaveLength(2);
		await completeRequest(0);
		expect(reports[2]).toEqual(b);
		await completeRequest(2);
		await reporter.idle();
		expect(saved).toEqual(['put 12', 'put 12']);
	});

	it('replaces a pending body and identity together without changing the bypass body', async () => {
		const { reporter, reports, completeRequest } = setup();
		reporter.flush(at(1000));
		reporter.flushBeforeUnload(at(2000));
		const bypass = structuredClone(reports[1]);
		const snapshot = at(3000.6, 102);
		reporter.flush(snapshot);
		snapshot.positionMs = 99_000;
		await completeRequest(1, new Error('offline'));
		await completeRequest(0);
		expect(reports[1]).toEqual(bypass);
		expect(reports[2]).toEqual({
			workId: 12, keepalive: true,
			body: {
				track_id: 102, position_ms: 3001, group_path: 'mp3/SE有',
				report_order: { session_id: reports[0]!.body.report_order!.session_id, sequence: 3 }
			}
		});
	});

	it('keeps one session across begin, tracks, works, pause and visibility changes', async () => {
		const { reporter, reports, complete, advance } = setup();
		reporter.tick(at(1000));
		await complete();
		reporter.tick(at(2000)); // Throttled reports consume no sequence.
		reporter.flush(at(1000)); // A new report at the same position still gets a new identity.
		await complete();
		const finished = reporter.markFinished(12, 101, 'mp3/SE有', { seal: true });
		await complete();
		await finished;
		reporter.flushBeforeUnload(at(2000)); // Sealed reports consume no sequence either.
		reporter.begin(12);
		reporter.flush(at(3000, 102));
		await complete();
		reporter.begin(13);
		reporter.flush({ ...at(4000, 103), workId: 13 });
		await complete();
		reporter.flushBeforeUnload(at(5000));
		await complete();
		reporter.begin(12); // Restoring the same page does not start a new account cycle.
		advance(REPORT_INTERVAL_MS);
		reporter.tick(at(6000));
		await complete();
		expect(reports.map((r) => r.body.report_order?.sequence)).toEqual([1, 2, 3, 4, 5, 6]);
		expect(new Set(reports.map((r) => r.body.report_order?.session_id)).size).toBe(1);
	});

	it('starts a new session after reset and for each new reporter, keeping in-flight bodies intact', async () => {
		const { reporter, reports, complete } = setup();
		reporter.flush(at(1000));
		const sent = structuredClone(reports[0]);
		reporter.flush(at(2000));
		reporter.reset();
		reporter.flush(at(3000));
		await complete();
		expect(reports[0]).toEqual(sent);
		expect(reports).toHaveLength(2);
		expect(reports[1]!.body.report_order?.sequence).toBe(1);
		expect(reports[1]!.body.report_order?.session_id).not.toBe(reports[0]!.body.report_order?.session_id);
		const other = setup();
		other.reporter.flush(at(1000));
		expect(other.reports[0]!.body.report_order?.sequence).toBe(1);
		expect(new Set([...reports, ...other.reports].map((r) => r.body.report_order?.session_id)).size).toBe(3);
	});

	it('uses the plain-HTTP random source only when a logical report is created', async () => {
		const getRandomValues = vi.fn((bytes: Uint8Array) => {
			bytes.set([0, 1, 2, 3, 4, 5, 6, 7, 248, 249, 250, 251, 252, 253, 254, 255]);
			return bytes;
		});
		vi.stubGlobal('crypto', { getRandomValues });
		const { reporter, reports, complete } = setup();
		reporter.begin(12);
		const cleared = reporter.clear(12);
		await complete();
		await cleared;
		reporter.flush(at(1000));
		expect(getRandomValues).not.toHaveBeenCalled();
		reporter.begin(12);
		reporter.flush(at(1000));
		expect(reports[0]!.body.report_order).toEqual({
			session_id: '0001020304050607f8f9fafbfcfdfeff', sequence: 1
		});
		await complete();
		reporter.flush(at(2000));
		expect(getRandomValues).toHaveBeenCalledOnce();
	});

	it('holds finish behind a bypass and retries its original body if the bypass fails', async () => {
		const { reporter, reports, calls, completeRequest } = setup();
		reporter.flush(at(1000));
		reporter.flushBeforeUnload(at(2000));
		const done = reporter.markFinished(12, 101, 'mp3/SE有', { seal: true });
		await completeRequest(0);
		expect(calls).toHaveLength(2);
		await completeRequest(1, new Error('offline'));
		expect(reports[2]).toEqual(reports[1]);
		expect(reports[2]!.body.report_order).toEqual({
			session_id: expect.stringMatching(/^[0-9a-f]{32}$/), sequence: 2
		});
		await completeRequest(2);
		expect(calls[3]).toBe('finish 12 101 mp3/SE有 manual');
		await completeRequest(3);
		await done;
	});

	it('announces each write that reached the server, and only those', async () => {
		const { reporter, saved, complete } = setup();
		reporter.flush(at(1000));
		await complete(new Error('offline'));
		expect(saved).toEqual([]);
		reporter.flush(at(2000));
		await complete();
		expect(saved).toEqual(['put 12']);
		reporter.trackEnded(at(3000), true);
		await complete();
		await complete();
		expect(saved).toEqual(['put 12', 'put 12', 'finish 12']);
	});

	it('reports at most once per interval while playing', async () => {
		const { reporter, calls, advance, complete } = setup();
		reporter.tick(at(1000));
		await complete();
		advance(REPORT_INTERVAL_MS - 1);
		reporter.tick(at(2000));
		expect(calls).toEqual(['put 12 101 1000']);
		advance(1);
		reporter.tick(at(3000));
		expect(calls).toEqual(['put 12 101 1000', 'put 12 101 3000']);
	});

	it('sends one request at a time and keeps only the newest waiting position', async () => {
		const { reporter, calls, complete } = setup();
		reporter.flush(at(1000));
		reporter.flush(at(2000));
		reporter.flush(at(3000));
		expect(calls).toEqual(['put 12 101 1000']);
		await complete();
		expect(calls).toEqual(['put 12 101 1000', 'put 12 101 3000']);
		await complete();
		await reporter.idle();
		expect(calls).toHaveLength(2);
	});

	it('keeps going after a failed report', async () => {
		const { reporter, calls, complete } = setup();
		reporter.flush(at(1000));
		reporter.flush(at(2000, 102));
		await complete(new Error('offline'));
		expect(calls).toEqual(['put 12 101 1000', 'put 12 102 2000']);
	});

	it('drops unsent reports on clear and sends the clear after the request in flight', async () => {
		const { reporter, calls, complete } = setup();
		reporter.flush(at(1000));
		reporter.flush(at(2000));
		const cleared = reporter.clear(12);
		await complete();
		expect(calls).toEqual(['put 12 101 1000', 'clear 12']);
		await complete();
		await expect(cleared).resolves.toBeUndefined();
	});

	it('ignores reports for a cleared work until its playback starts again', async () => {
		const { reporter, calls, complete, advance } = setup();
		void reporter.clear(12);
		await complete();
		advance(REPORT_INTERVAL_MS);
		reporter.flush(at(5000));
		reporter.tick(at(6000));
		expect(calls).toEqual(['clear 12']);
		reporter.begin(12);
		reporter.flush(at(7000));
		expect(calls).toEqual(['clear 12', 'put 12 101 7000']);
	});

	it('finishes only when the last track of the group ends', async () => {
		const { reporter, calls, complete } = setup();
		reporter.trackEnded(at(301000, 101), false);
		await complete();
		expect(calls).toEqual(['put 12 101 301000']);
		reporter.trackEnded(at(1995000, 102), true);
		await complete();
		expect(calls).toEqual(['put 12 101 301000', 'put 12 102 1995000', 'finish 12 102 mp3/SE有']);
	});

	it('does not report again after finishing, so the work is not marked as replaying', async () => {
		const { reporter, calls, complete } = setup();
		reporter.trackEnded(at(1995000, 102), true);
		await complete();
		await complete();
		reporter.flush(at(1995000, 102));
		expect(calls).toEqual(['put 12 102 1995000', 'finish 12 102 mp3/SE有']);
	});

	it('sends a manual finish through the same queue and reports its outcome', async () => {
		const { reporter, calls, complete } = setup();
		reporter.flush(at(1000));
		const done = reporter.markFinished(12, 101, 'mp3/SE有');
		await complete();
		expect(calls).toEqual(['put 12 101 1000', 'finish 12 101 mp3/SE有 manual']);
		const outcome = expect(done).rejects.toThrow('bad request');
		await complete(new Error('bad request'));
		await outcome;
	});

	it('sends the last report before unload at once with keepalive', async () => {
		const { reporter, calls } = setup();
		reporter.flushBeforeUnload(at(4000));
		expect(calls).toEqual(['put 12 101 4000 keepalive']);
	});

	it('does not leave the unload report waiting behind a request in flight', async () => {
		const { reporter, calls, complete } = setup();
		reporter.flush(at(1000));
		reporter.flushBeforeUnload(at(4000));
		// Sent right away, without waiting for the first request.
		expect(calls).toEqual(['put 12 101 1000', 'put 12 101 4000 keepalive']);
		await complete();
		await complete();
		// A surviving page also sends the queued copy for retry and save notification.
		expect(calls).toEqual([
			'put 12 101 1000',
			'put 12 101 4000 keepalive',
			'put 12 101 4000 keepalive'
		]);
	});

	it('holds a later clear until the unload report sent past the queue is done', async () => {
		const { reporter, calls, complete } = setup();
		reporter.flush(at(1000));
		reporter.flushBeforeUnload(at(4000));
		// The page comes back and the user clears the progress.
		const cleared = reporter.clear(12);
		await complete();
		expect(calls).toEqual(['put 12 101 1000', 'put 12 101 4000 keepalive']);
		await complete();
		expect(calls.at(-1)).toBe('clear 12');
		await complete();
		await expect(cleared).resolves.toBeUndefined();
	});

	it('can seal a work on a manual finish until its playback starts again', async () => {
		const { reporter, calls, complete } = setup();
		const done = reporter.markFinished(12, 101, 'mp3/SE有', { seal: true });
		await complete();
		await done;
		reporter.flushBeforeUnload(at(5000));
		reporter.flush(at(5000));
		expect(calls).toEqual(['finish 12 101 mp3/SE有 manual']);
		reporter.begin(12);
		reporter.flush(at(6000));
		expect(calls.at(-1)).toBe('put 12 101 6000');
	});

	it('drops a report waiting behind the unload report when the account changes', async () => {
		const { reporter, calls, complete } = setup();
		reporter.flush(at(1000));
		reporter.flushBeforeUnload(at(4000));
		await complete();
		// The queued copy now waits for the keepalive write; the user signs out.
		reporter.reset();
		await complete();
		await reporter.idle();
		expect(calls).toEqual(['put 12 101 1000', 'put 12 101 4000 keepalive']);
	});

	it('forgets everything queued when the account changes', async () => {
		const { reporter, calls, complete } = setup();
		reporter.flush(at(1000));
		reporter.flush(at(2000));
		const cleared = expect(reporter.clear(13)).rejects.toThrow();
		reporter.reset();
		await complete();
		await reporter.idle();
		expect(calls).toEqual(['put 12 101 1000']);
		await cleared;
	});
});
