import { describe, expect, it, vi } from 'vitest';
import loudnessSample from '../../../../contract/loudness/loudness.json';
import pendingSample from '../../../../contract/loudness/loudness-processing.json';
import detailSample from '../../../../contract/m2/work-detail.json';
import taskDone from '../../../../contract/m2/task-done.json';
import taskRunning from '../../../../contract/m2/task-running.json';
import type { AnalysisPending } from '../api/index';
import type { Loudness, TaskSnapshot, TrackItem, WorkDetail } from '../api/types';
import {
	MAX_ANALYSIS_ROUNDS,
	TrackAnalysis,
	untilAnalysed,
	type TrackAnalysisDeps
} from './analysis.svelte';
import { DEFAULT_USER_SETTINGS, trackGain } from './loudness';
import { POLL_INTERVAL_MS, abortError } from './prepare';
import { WAVEFORM_PEAKS } from './waveform';

const measured = loudnessSample as Loudness;
const pending = pendingSample as AnalysisPending;
const analysisTask = (state: TaskSnapshot['state']): TaskSnapshot => ({
	...(taskRunning as TaskSnapshot),
	id: pending.task_id,
	kind: 'analysis',
	state,
	result: null
});
const running = analysisTask('running');
const done = { ...(taskDone as TaskSnapshot), id: pending.task_id, kind: 'analysis', result: null } as TaskSnapshot;

const tracks = (detailSample as WorkDetail).tracks;
const track = (id: number): TrackItem => tracks.find((t) => t.id === id)!;
/** Tr01 comes with a measured loudness, Tr02 and the wav do not, Tr02.wav is missing. */
const measuredTrack = track(101);
const unmeasured = track(102);
const otherUnmeasured = track(104);
const missingTrack = track(105);
const videoTrack: TrackItem = { ...unmeasured, id: 201, path: 'video/op.mp4', kind: 'video' };

/** A sleep that resolves on the next tick, or rejects once aborted. */
function instantSleep(ms: number, signal: AbortSignal): Promise<void> {
	void ms;
	return new Promise((resolve, reject) => {
		if (signal.aborted) return reject(abortError());
		setTimeout(() => (signal.aborted ? reject(abortError()) : resolve()), 0);
	});
}

/** A promise the test resolves by hand. */
function deferred<T>() {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((r) => (resolve = r));
	return { promise, resolve };
}

const settle = async () => {
	for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
};

describe('untilAnalysed', () => {
	const signal = new AbortController().signal;

	it('returns a 200 answer at once', async () => {
		const getTask = vi.fn();
		const result = await untilAnalysed(async () => measured, signal, { getTask, sleep: instantSleep });
		expect(result).toEqual(measured);
		expect(getTask).not.toHaveBeenCalled();
	});

	it('polls the task of a 202 answer until it is done, then asks again', async () => {
		const answers: (Loudness | AnalysisPending)[] = [pending, measured];
		const request = vi.fn(async () => answers.shift()!);
		const tasks = [running, running, done];
		const getTask = vi.fn(async () => tasks.shift()!);
		const sleep = vi.fn(instantSleep);
		const result = await untilAnalysed(request, signal, { getTask, sleep });
		expect(result).toEqual(measured);
		expect(request).toHaveBeenCalledTimes(2);
		expect(getTask).toHaveBeenCalledTimes(3);
		expect(getTask).toHaveBeenCalledWith(pending.task_id);
		expect(sleep.mock.calls.every(([ms]) => ms === POLL_INTERVAL_MS)).toBe(true);
	});

	it('returns a waveform body, which is not mistaken for a 202 answer', async () => {
		const body = new Uint8Array(WAVEFORM_PEAKS);
		const result = await untilAnalysed(async () => body, signal, { getTask: vi.fn(), sleep: instantSleep });
		expect(result).toBe(body);
	});

	it.each(['failed', 'cancelled', 'interrupted'] as const)('gives null when the task is %s', async (state) => {
		const request = vi.fn(async () => pending);
		const result = await untilAnalysed(request, signal, {
			getTask: async () => analysisTask(state),
			sleep: instantSleep
		});
		expect(result).toBeNull();
		expect(request).toHaveBeenCalledTimes(1);
	});

	it(`gives up after ${MAX_ANALYSIS_ROUNDS} rounds of 202 answers`, async () => {
		const request = vi.fn(async () => pending);
		const result = await untilAnalysed(request, signal, { getTask: async () => done, sleep: instantSleep });
		expect(result).toBeNull();
		expect(request).toHaveBeenCalledTimes(MAX_ANALYSIS_ROUNDS);
	});

	it('gives null when a request fails', async () => {
		const failing = untilAnalysed(
			async () => {
				throw new Error('network');
			},
			signal,
			{ getTask: vi.fn(), sleep: instantSleep }
		);
		await expect(failing).resolves.toBeNull();

		const failingTask = untilAnalysed(async () => pending, signal, {
			getTask: async () => {
				throw new Error('network');
			},
			sleep: instantSleep
		});
		await expect(failingTask).resolves.toBeNull();
	});

	it('stops polling and gives null once aborted', async () => {
		const controller = new AbortController();
		const getTask = vi.fn(async () => {
			if (getTask.mock.calls.length === 2) controller.abort();
			return running;
		});
		const result = await untilAnalysed(async () => pending, controller.signal, {
			getTask,
			sleep: instantSleep
		});
		expect(result).toBeNull();
		expect(getTask).toHaveBeenCalledTimes(2);
	});

	it('drops an answer that arrives after the abort', async () => {
		const controller = new AbortController();
		const answer = deferred<Loudness>();
		const result = untilAnalysed(() => answer.promise, controller.signal, {
			getTask: vi.fn(),
			sleep: instantSleep
		});
		controller.abort();
		answer.resolve(measured);
		await expect(result).resolves.toBeNull();
	});
});

function fakeDeps() {
	return {
		getLoudness: vi.fn<TrackAnalysisDeps['getLoudness']>(async () => measured),
		getWaveform: vi.fn<TrackAnalysisDeps['getWaveform']>(async () => new Uint8Array(WAVEFORM_PEAKS).fill(255)),
		getTask: vi.fn<TrackAnalysisDeps['getTask']>(async () => done),
		sleep: vi.fn<TrackAnalysisDeps['sleep']>(instantSleep)
	};
}

describe('TrackAnalysis.follow', () => {
	it('does not ask for the waveform when the layout does not show it', async () => {
		const deps = fakeDeps();
		const analysis = new TrackAnalysis(deps);
		analysis.follow(unmeasured, { waveform: false, loudness: false, prepared: true });
		await settle();
		expect(deps.getWaveform).not.toHaveBeenCalled();
		expect(analysis.waveform).toBeNull();
	});

	it('shows the waveform of the playing track once it arrives', async () => {
		const deps = fakeDeps();
		const analysis = new TrackAnalysis(deps);
		analysis.follow(unmeasured, { waveform: true, loudness: false, prepared: true });
		await settle();
		expect(deps.getWaveform).toHaveBeenCalledWith(unmeasured.id);
		expect(analysis.waveform?.trackId).toBe(unmeasured.id);
		expect(analysis.waveform?.peaks).toHaveLength(WAVEFORM_PEAKS);

		// Following the same track again asks for nothing new.
		analysis.follow(unmeasured, { waveform: true, loudness: false, prepared: true });
		await settle();
		expect(deps.getWaveform).toHaveBeenCalledTimes(1);
	});

	it.each([
		['a video', videoTrack],
		['a missing track', missingTrack],
		['no track', null]
	])('never asks anything for %s', async (_name, item) => {
		const deps = fakeDeps();
		const analysis = new TrackAnalysis(deps);
		analysis.follow(item, { waveform: true, loudness: true, prepared: true });
		await settle();
		expect(deps.getWaveform).not.toHaveBeenCalled();
		expect(deps.getLoudness).not.toHaveBeenCalled();
		expect(analysis.loudnessOf(item)).toBeNull();
	});

	it('leaves a video at its own level even when it carries a loudness', () => {
		const analysis = new TrackAnalysis(fakeDeps());
		expect(analysis.loudnessOf({ ...videoTrack, loudness: measured })).toBeNull();
	});

	it('asks for loudness only while normalisation is on and the track is unmeasured', async () => {
		const deps = fakeDeps();
		const analysis = new TrackAnalysis(deps);
		analysis.follow(unmeasured, { waveform: false, loudness: false, prepared: true });
		analysis.follow(measuredTrack, { waveform: false, loudness: true, prepared: true });
		await settle();
		expect(deps.getLoudness).not.toHaveBeenCalled();
		expect(analysis.loudnessOf(measuredTrack)).toEqual(measuredTrack.loudness);

		analysis.follow(unmeasured, { waveform: false, loudness: true, prepared: true });
		await settle();
		expect(deps.getLoudness).toHaveBeenCalledTimes(1);
		expect(deps.getLoudness).toHaveBeenCalledWith(unmeasured.id);
	});

	it('returns the measured loudness after it arrives, and does not ask for it again', async () => {
		const deps = fakeDeps();
		deps.getLoudness.mockResolvedValueOnce(pending);
		const analysis = new TrackAnalysis(deps);
		analysis.follow(unmeasured, { waveform: false, loudness: true, prepared: true });
		expect(analysis.loudnessOf(unmeasured)).toBeNull();
		await settle();
		expect(deps.getTask).toHaveBeenCalledWith(pending.task_id);
		expect(analysis.loudnessOf(unmeasured)).toEqual(measured);

		analysis.follow(otherUnmeasured, { waveform: false, loudness: true, prepared: true });
		analysis.follow(unmeasured, { waveform: false, loudness: true, prepared: true });
		await settle();
		expect(deps.getLoudness).toHaveBeenCalledTimes(3);
		expect(deps.getLoudness.mock.calls.map(([id]) => id)).toEqual([
			unmeasured.id,
			unmeasured.id,
			otherUnmeasured.id
		]);
	});

	it.each([
		['after switching tracks', true],
		['while the same track is selected', false]
	])('revalidates loudness from a fresh detail %s', async (_name, switchTracks) => {
		const deps = fakeDeps();
		const oldLoudness: Loudness = { integrated_lufs: -30, true_peak_dbtp: -10 };
		const newLoudness: Loudness = { integrated_lufs: -20, true_peak_dbtp: 0 };
		deps.getLoudness.mockResolvedValueOnce(oldLoudness);
		const analysis = new TrackAnalysis(deps);
		const options = { waveform: false, loudness: true, prepared: true };
		const settings = { ...DEFAULT_USER_SETTINGS, normalize_loudness: true };
		analysis.follow(unmeasured, options);
		await settle();
		expect(trackGain(analysis.loudnessOf(unmeasured), settings)).toBeCloseTo(2.81838, 5);
		if (switchTracks) analysis.follow(measuredTrack, options);

		// A same-size replacement changes mtime, which the detail does not expose.
		// The server invalidates its old measurement and returns loudness: null.
		const refreshed: TrackItem = { ...unmeasured, loudness: null };
		const answer = deferred<Loudness>();
		deps.getLoudness.mockReturnValueOnce(answer.promise);
		expect(analysis.loudnessOf(refreshed)).toBeNull();
		analysis.follow(refreshed, options);
		expect(deps.getLoudness).toHaveBeenCalledTimes(2);
		expect(deps.getLoudness).toHaveBeenLastCalledWith(refreshed.id);
		expect(trackGain(analysis.loudnessOf(refreshed), settings)).toBe(1);

		answer.resolve(newLoudness);
		await settle();
		expect(analysis.loudnessOf(refreshed)).toEqual(newLoudness);
		// The new 0 dBTP peak limits the gain to -1 dB, not the requested +4 dB.
		expect(trackGain(analysis.loudnessOf(refreshed), settings)).toBeCloseTo(0.89125, 5);
	});

	it('stops polling for the previous track when the track changes', async () => {
		const deps = fakeDeps();
		deps.getLoudness.mockResolvedValueOnce(pending);
		const task = deferred<TaskSnapshot>();
		deps.getTask.mockReturnValueOnce(task.promise);
		const analysis = new TrackAnalysis(deps);
		analysis.follow(unmeasured, { waveform: false, loudness: true, prepared: true });
		await settle();
		expect(deps.getTask).toHaveBeenCalledTimes(1);

		analysis.follow(otherUnmeasured, { waveform: false, loudness: true, prepared: true });
		task.resolve(done);
		await settle();
		// The old track neither polls again nor asks for its loudness again.
		expect(deps.getTask).toHaveBeenCalledTimes(1);
		expect(deps.getLoudness.mock.calls.map(([id]) => id)).toEqual([unmeasured.id, otherUnmeasured.id]);
		expect(analysis.loudnessOf(unmeasured)).toBeNull();
		expect(analysis.loudnessOf(otherUnmeasured)).toEqual(measured);
	});

	it('never shows a late answer for the previous track', async () => {
		const deps = fakeDeps();
		const loud = deferred<Loudness>();
		const wave = deferred<Uint8Array>();
		deps.getLoudness.mockReturnValueOnce(loud.promise);
		deps.getWaveform.mockReturnValueOnce(wave.promise);
		const nextWave = deferred<Uint8Array>();
		deps.getWaveform.mockReturnValueOnce(nextWave.promise);
		const analysis = new TrackAnalysis(deps);
		analysis.follow(unmeasured, { waveform: true, loudness: true, prepared: true });
		analysis.follow(otherUnmeasured, { waveform: true, loudness: true, prepared: true });

		loud.resolve(measured);
		wave.resolve(new Uint8Array(WAVEFORM_PEAKS).fill(255));
		await settle();
		expect(analysis.loudnessOf(unmeasured)).toBeNull();
		expect(analysis.waveform).toBeNull();

		nextWave.resolve(new Uint8Array(WAVEFORM_PEAKS));
		await settle();
		expect(analysis.waveform?.trackId).toBe(otherUnmeasured.id);
	});

	it('clears the waveform when the track changes', async () => {
		const deps = fakeDeps();
		const analysis = new TrackAnalysis(deps);
		analysis.follow(unmeasured, { waveform: true, loudness: false, prepared: true });
		await settle();
		expect(analysis.waveform?.trackId).toBe(unmeasured.id);

		deps.getWaveform.mockReturnValueOnce(deferred<Uint8Array>().promise);
		analysis.follow(otherUnmeasured, { waveform: true, loudness: false, prepared: true });
		expect(analysis.waveform).toBeNull();
	});
});

describe('TrackAnalysis.reset', () => {
	it('forgets measured loudness and the waveform', async () => {
		const deps = fakeDeps();
		const analysis = new TrackAnalysis(deps);
		analysis.follow(unmeasured, { waveform: true, loudness: true, prepared: true });
		await settle();
		expect(analysis.loudnessOf(unmeasured)).toEqual(measured);
		expect(analysis.waveform).not.toBeNull();

		analysis.reset();
		expect(analysis.loudnessOf(unmeasured)).toBeNull();
		expect(analysis.waveform).toBeNull();
	});

	it('stops the requests that are still out', async () => {
		const deps = fakeDeps();
		const loud = deferred<Loudness>();
		deps.getLoudness.mockReturnValueOnce(loud.promise);
		const analysis = new TrackAnalysis(deps);
		analysis.follow(unmeasured, { waveform: false, loudness: true, prepared: true });
		analysis.reset();
		loud.resolve(measured);
		await settle();
		expect(analysis.loudnessOf(unmeasured)).toBeNull();
	});

	it('waits for prepare before asking anything for the track', async () => {
		const deps = fakeDeps();
		const analysis = new TrackAnalysis(deps);
		analysis.follow(unmeasured, { waveform: true, loudness: true, prepared: false });
		await settle();
		expect(deps.getWaveform).not.toHaveBeenCalled();
		expect(deps.getLoudness).not.toHaveBeenCalled();

		analysis.follow(unmeasured, { waveform: true, loudness: true, prepared: true });
		await settle();
		expect(deps.getWaveform).toHaveBeenCalledTimes(1);
		expect(deps.getLoudness).toHaveBeenCalledTimes(1);
	});

	it('measures a track again when its file changed', async () => {
		const deps = fakeDeps();
		const analysis = new TrackAnalysis(deps);
		analysis.follow(unmeasured, { waveform: false, loudness: true, prepared: true });
		await settle();
		expect(analysis.loudnessOf(unmeasured)).toEqual(measured);

		// A refreshed work detail lists the same track with another size.
		const replaced = { ...unmeasured, size: (unmeasured.size ?? 0) + 1 };
		expect(analysis.loudnessOf(replaced)).toBeNull();
		analysis.follow(replaced, { waveform: false, loudness: true, prepared: true });
		await settle();
		expect(deps.getLoudness).toHaveBeenCalledTimes(2);
	});
});
