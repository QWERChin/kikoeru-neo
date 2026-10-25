// Mock answers for loudness, waveforms and the user settings (contract
// "响度和波形"). Like the server, an unmeasured track answers 202 with an
// `analysis` task, and both endpoints answer 200 once the task is done.
import loudnessSample from '../../../../contract/loudness/loudness.json';
import loudnessProcessing from '../../../../contract/loudness/loudness-processing.json';
import type { ApiRequest, ApiResponse } from './transport';
import type { ErrorCode, ErrorResponse, Loudness, TaskSnapshot, TrackItem, UserSettings } from './types';

/** Polls of a mock analysis task that answer `running` before it is done. */
export const MOCK_ANALYSIS_RUNNING_POLLS = 2;

/** Sample track 101 (Tr01.mp3) counts as measured, as in the work detail sample. */
const MEASURED_SAMPLE_TRACK = 101;
const TRACK_ID_STRIDE = 1000;

/**
 * What the mock measures for every other track: quiet enough that the -1
 * dBTP limit caps the boost at the default target.
 */
export const MOCK_MEASURED_LOUDNESS: Loudness = { integrated_lufs: -23, true_peak_dbtp: -3.5 };

const DEFAULT_SETTINGS: UserSettings = {
	normalize_loudness: false,
	target_lufs: -16,
	accent: null,
	seek_back_seconds: 15,
	seek_forward_seconds: 30
};
const ACCENTS = ['blue', 'purple', 'pink', 'red', 'orange', 'green', 'teal', 'indigo'];
let settings: UserSettings = { ...DEFAULT_SETTINGS };

const measured = new Set<number>();
const analyses = new Map<number, { trackId: number; polls: number }>();
// Far from the transcode tasks (from 88) and the scan tasks (from 10000).
let nextTaskId = 5000;

function errorResponse(status: number, code: ErrorCode, message: string): ApiResponse {
	return { status, body: { error: { code, message } } satisfies ErrorResponse };
}

function isMeasured(trackId: number): boolean {
	return trackId % TRACK_ID_STRIDE === MEASURED_SAMPLE_TRACK || measured.has(trackId);
}

function loudnessOf(trackId: number): Loudness {
	return trackId % TRACK_ID_STRIDE === MEASURED_SAMPLE_TRACK ? loudnessSample : MOCK_MEASURED_LOUDNESS;
}

/** Peaks that rise and fall with the track id, so each track looks different. */
export function mockWaveform(trackId: number): Uint8Array {
	const bytes = new Uint8Array(1000);
	for (let i = 0; i < bytes.length; i++) {
		const swell = 0.55 + 0.45 * Math.sin((i + trackId * 17) / 90);
		const ripple = 0.6 + 0.4 * Math.abs(Math.sin((i * (3 + (trackId % 5))) / 11));
		bytes[i] = Math.max(1, Math.round(255 * swell * ripple * 0.9));
	}
	return bytes;
}

function queueAnalysis(trackId: number): ApiResponse {
	// Like the server's dedupe key, one task per track.
	const existing = [...analyses].find(([, task]) => task.trackId === trackId);
	const taskId = existing ? existing[0] : nextTaskId++;
	if (!existing) analyses.set(taskId, { trackId, polls: 0 });
	return { status: 202, body: { ...loudnessProcessing, task_id: taskId } };
}

/** Polls an analysis task, or returns null when the id belongs to another kind. */
export function pollAnalysisTask(taskId: number): ApiResponse | null {
	const task = analyses.get(taskId);
	if (!task) return null;
	task.polls += 1;
	const done = task.polls > MOCK_ANALYSIS_RUNNING_POLLS;
	// The task stays, because the loudness and the waveform poll it separately
	// and both must see it finish.
	if (done) measured.add(task.trackId);
	const now = Date.now();
	const snapshot: TaskSnapshot = {
		id: taskId,
		kind: 'analysis',
		state: done ? 'done' : 'running',
		progress: done ? 1 : task.polls / (MOCK_ANALYSIS_RUNNING_POLLS + 1),
		message: null,
		result: null,
		queue_position: null,
		created_at: now,
		updated_at: now
	};
	return { status: 200, body: snapshot };
}

function handleTrack(track: TrackItem | null, what: 'loudness' | 'waveform'): ApiResponse {
	if (!track) return errorResponse(404, 'not_found', '没有这条轨道');
	// Like the server, only audio is analysed, so a video is not_applicable too.
	if (track.kind !== 'audio') return errorResponse(404, 'not_applicable', '只分析音频');
	if (track.missing) return errorResponse(410, 'missing', '这个文件已经找不到了');
	if (!isMeasured(track.id)) return queueAnalysis(track.id);
	return what === 'loudness'
		? { status: 200, body: { ...loudnessOf(track.id) } }
		: { status: 200, body: mockWaveform(track.id) };
}

function handleSettings(method: string, body: unknown): ApiResponse | null {
	if (method === 'GET') return { status: 200, body: { ...settings } };
	if (method !== 'PUT') return null;
	const next = (body ?? {}) as Partial<UserSettings>;
	const target = next.target_lufs;
	if (
		typeof next.normalize_loudness !== 'boolean' ||
		typeof target !== 'number' ||
		!Number.isFinite(target) ||
		target < -30 ||
		target > -10
	) {
		return errorResponse(400, 'invalid_request', '目标响度要在 -30 到 -10 LUFS 之间');
	}
	const accent = next.accent ?? null;
	if (accent !== null && !ACCENTS.includes(accent)) {
		return errorResponse(400, 'invalid_request', '没有这个强调色');
	}
	const back = next.seek_back_seconds ?? DEFAULT_SETTINGS.seek_back_seconds;
	const forward = next.seek_forward_seconds ?? DEFAULT_SETTINGS.seek_forward_seconds;
	if (![back, forward].every((s) => Number.isInteger(s) && s >= 1 && s <= 99)) {
		return errorResponse(400, 'invalid_request', '快退和快进的秒数要在 1 到 99 之间');
	}
	settings = {
		normalize_loudness: next.normalize_loudness,
		target_lufs: target,
		accent,
		seek_back_seconds: back,
		seek_forward_seconds: forward
	};
	return { status: 200, body: { ...settings } };
}

export function routeAnalysis(
	request: ApiRequest,
	findTrack: (trackId: number) => TrackItem | null
): ApiResponse | null {
	const { method, path, body } = request;
	if (path === '/me/settings') return handleSettings(method, body);
	const m = /^\/tracks\/(\d+)\/(loudness|waveform)$/.exec(path);
	if (m && method === 'GET') return handleTrack(findTrack(Number(m[1])), m[2] as 'loudness' | 'waveform');
	return null;
}
