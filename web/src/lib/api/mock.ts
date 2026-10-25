// Serves the contract samples so pages can be built before the backend
// exists. Loaded only when VITE_API_MOCK=1 (see client.ts). Media bytes and
// covers are not JSON, so the dev server answers those (see mock-media.ts).
import errorSetupRequired from '../../../../contract/m1/error-setup-required.json';
import errorUnauthorized from '../../../../contract/m1/error-unauthorized.json';
import loginResponse from '../../../../contract/m1/login-response.json';
import mediaTokenResponse from '../../../../contract/m1/media-token.json';
import meResponse from '../../../../contract/m1/me.json';
import versionResponse from '../../../../contract/m1/version.json';
import prepareProcessing from '../../../../contract/m2/prepare-processing.json';
import prepareReady from '../../../../contract/m2/prepare-ready.json';
import scanResponse from '../../../../contract/m2/scan-response.json';
import refreshResponse from '../../../../contract/m4/refresh-response.json';
import taskDone from '../../../../contract/m2/task-done.json';
import taskRunning from '../../../../contract/m2/task-running.json';
import workDetailSample from '../../../../contract/m2/work-detail.json';
import fontsSample from '../../../../contract/m6/fonts.json';
import prepareReadyHls from '../../../../contract/m6/prepare-ready-hls.json';
import taskDoneHls from '../../../../contract/m6/task-done-hls.json';
import trackVideoSample from '../../../../contract/m6/track-video.json';
import type { ApiRequest, ApiResponse, StreamTransport, Transport } from './transport';
import type { ErrorCode, ErrorResponse, Progress, TrackFont, User, WorkSummary } from './types';
import type { CircleRef, MetadataRequest, PrepareRequest, ScanRequest, TagRef, VaRef } from './types';
import type { CleanupRequest, TagMergeRequest, TaskSnapshot, TrackItem, WorkDetail } from './types';
import * as admin from './mock-admin';
import { routeRoots, rootsState } from './mock-roots';
import { pollAnalysisTask, routeAnalysis } from './mock-analysis';
import { myRating, myReview, routeReviews } from './mock-reviews';
import { routeSettings } from './mock-settings';
import { MOCK_ASS } from './mock-ass';
import { MOCK_TEASER_TRACK_ID, MOCK_VIDEO_SECONDS, mockMediaToken } from './mock-shared';
import {
	cancelMockTask,
	moveMockTaskToFront,
	getMockTask,
	isTerminalState,
	listMockTasks,
	scanRunning,
	startRefresh,
	startScans
} from './mock-tasks';
import { MOCK_SUBTITLE } from './mock-subtitle';
import { MOCK_DEFAULT_TRACK_SECONDS } from './mock-durations';
import {
	MOCK_DOWN_ROOT,
	MOCK_STORAGE_DOWN_QUERY,
	catalog,
	circles,
	favoriteOrder,
	historyOrder,
	moveToFront,
	pageParams,
	paginate,
	queryWorks,
	removeFrom,
	tagRefs,
	tags,
	vaRefs,
	vas,
	type MockWork
} from './mock-catalog';

const MIN_LATENCY_MS = 120;
const MAX_LATENCY_MS = 400;

/** Logging in with this name returns the setup_required sample. */
export const MOCK_SETUP_REQUIRED_NAME = 'setup';
/** Logging in with this name returns too_many_attempts. */
export const MOCK_RATE_LIMITED_NAME = 'locked';

/** Changing the password with this as the current one answers invalid_credentials. */
export const MOCK_WRONG_CURRENT_PASSWORD = 'wrong password';

/** Logging in with this name signs in a listener without admin rights. */
export const MOCK_LISTENER_NAME = 'listener';
const LISTENER: User = { id: 2, name: MOCK_LISTENER_NAME, is_admin: false };
const LISTENER_TOKEN = 'mock-listener-token';

/** Polls of a mock transcode task that answer `running` before it is done. */
export const MOCK_TASK_RUNNING_POLLS = 2;

/** The sample's track ids belong to work 12; other works get shifted copies. */
const SAMPLE_WORK_ID = workDetailSample.id;
const TRACK_ID_STRIDE = 1000;

function errorResponse(status: number, code: ErrorCode, message: string): ApiResponse {
	return { status, body: { error: { code, message } } satisfies ErrorResponse };
}

function noContent(): ApiResponse {
	return { status: 204, body: null };
}

function handleLogin(body: unknown): ApiResponse {
	const { name, password } = (body ?? {}) as { name?: unknown; password?: unknown };
	if (name === MOCK_SETUP_REQUIRED_NAME) return { status: 401, body: errorSetupRequired };
	if (name === MOCK_RATE_LIMITED_NAME) {
		return errorResponse(429, 'too_many_attempts', '登录失败次数太多，请稍后再试');
	}
	if (typeof password !== 'string' || password === '') {
		return errorResponse(401, 'invalid_credentials', '用户名或密码不对');
	}
	if (name === MOCK_LISTENER_NAME) {
		return { status: 200, body: { ...loginResponse, token: LISTENER_TOKEN, user: LISTENER } };
	}
	return { status: 200, body: loginResponse };
}

// Any other current password is right. The mock keeps no passwords, so the
// tokens it handed out stay valid, unlike on the real server.
function handlePasswordChange(body: unknown): ApiResponse {
	const { current_password, new_password } = (body ?? {}) as {
		current_password?: unknown;
		new_password?: unknown;
	};
	if (typeof new_password !== 'string' || [...new_password].length < 8) {
		return errorResponse(400, 'invalid_request', '新密码至少要 8 个字符');
	}
	if (typeof current_password !== 'string' || current_password === MOCK_WRONG_CURRENT_PASSWORD) {
		return errorResponse(401, 'invalid_credentials', '用户名或密码不对');
	}
	return noContent();
}

// Progress written through the M2 endpoints, so the pages reflect what the
// player reported until the browser reloads the mock.
const progressOverrides = new Map<number, Progress | null>();

interface ManualMetadata {
	title: string;
	release_date: string | null;
	nsfw: boolean;
	circle: CircleRef | null;
	vas: VaRef[];
	tags: TagRef[];
}

// Metadata saved through PUT /works/{id}/metadata. A plain refresh keeps it,
// a forced one drops it, which brings back the "DLsite" data of the samples.
const manualMetadata = new Map<number, ManualMetadata>();

/** The summary as the API would answer it now, with toggles and progress applied. */
function currentSummary(work: MockWork): WorkSummary {
	const { id } = work.summary;
	const manual = manualMetadata.get(id);
	return {
		...work.summary,
		...(manual && {
			title: manual.title,
			release_date: manual.release_date,
			nsfw: manual.nsfw,
			circle: manual.circle
		}),
		favorite: favoriteOrder.includes(id),
		progress: progressOverrides.has(id) ? (progressOverrides.get(id) ?? null) : work.summary.progress,
		my_rating: myRating(id)
	};
}

const byId = new Map(catalog.map((w) => [w.summary.id, w]));

function storageDown(): ApiResponse {
	return errorResponse(503, 'storage_unavailable', `媒体库 ${MOCK_DOWN_ROOT.name} 暂时无法访问`);
}

/** The work with its manual metadata applied, so search, filters and sorts see the edit. */
function effectiveWork(work: MockWork): MockWork {
	const manual = manualMetadata.get(work.summary.id);
	if (!manual) return work;
	return {
		...work,
		summary: currentSummary(work),
		tagIds: manual.tags.map((t) => t.id),
		vaIds: manual.vas.map((v) => v.id)
	};
}

function handleWorks(query: ApiRequest['query']): ApiResponse {
	if (String(query?.q ?? '').trim() === MOCK_STORAGE_DOWN_QUERY) return storageDown();
	const result = queryWorks(catalog.map(effectiveWork), query);
	if (!result.ok) return errorResponse(400, 'invalid_request', result.message);
	return { status: 200, body: paginate(result.works.map(currentSummary), pageParams(query)) };
}

function handleUserList(order: number[], query: ApiRequest['query']): ApiResponse {
	const works = order.flatMap((id) => {
		const work = byId.get(id);
		return work ? [currentSummary(work)] : [];
	});
	return { status: 200, body: paginate(works, pageParams(query)) };
}

function handleFavorite(method: string, workId: number): ApiResponse | null {
	if (!byId.has(workId)) return errorResponse(404, 'not_found', '没有这部作品');
	if (method === 'PUT') {
		if (!favoriteOrder.includes(workId)) favoriteOrder.unshift(workId);
		return noContent();
	}
	if (method === 'DELETE') {
		removeFrom(favoriteOrder, workId);
		return noContent();
	}
	return null;
}

/**
 * The M6 video sample, a direct-play mp4 beside it and the font files the
 * fonts sample lists, added to the M2 sample work. The generated videos are
 * shorter than the sample says, so the durations match the files.
 */
const VIDEO_TRACKS: TrackItem[] = [
	{ ...(trackVideoSample as TrackItem), duration_ms: MOCK_VIDEO_SECONDS * 1000 },
	{
		...(trackVideoSample as TrackItem),
		id: MOCK_TEASER_TRACK_ID,
		path: '映像/予告.mp4',
		size: 2_400_000,
		duration_ms: MOCK_VIDEO_SECONDS * 1000,
		has_subtitle: true,
		subtitle_formats: ['vtt'],
		video: { codec: 'h264', width: 640, height: 360 }
	},
	...fontsSample.map(
		(font): TrackItem => ({
			id: font.id,
			path: `映像/fonts/${font.name}`,
			kind: 'other',
			size: 145_972,
			duration_ms: null,
			has_subtitle: false,
			subtitle_formats: [],
			missing: false,
			video: null,
			loudness: null
		})
	)
];

const SAMPLE_DETAIL: WorkDetail = {
	...(workDetailSample as WorkDetail),
	tracks: [...(workDetailSample as WorkDetail).tracks, ...VIDEO_TRACKS]
};

// Far from the sample's track ids and from other works' (TRACK_ID_STRIDE apart).
const LARGE_FOLDER_FIRST_ID = 900_000;

/** Only admins see locations, so the listener gets `null` like on the server. */
function workDetail(workId: number, asAdmin = true): WorkDetail | null {
	const detail = workDetailForAdmin(workId);
	return detail && !asAdmin ? { ...detail, locations: null } : detail;
}

function workDetailForAdmin(workId: number): WorkDetail | null {
	const work = byId.get(workId);
	if (!work) return null;
	const summary = currentSummary(work);
	const sample = SAMPLE_DETAIL;
	const manual = manualMetadata.get(workId);
	if (workId === SAMPLE_WORK_ID) {
		// The detail sample has its own progress, which differs from the works page's.
		const progress = progressOverrides.has(workId) ? (progressOverrides.get(workId) ?? null) : sample.progress;
		const edited = manual && { ...summaryFields(summary), vas: manual.vas, tags: manual.tags };
		// Tags come from the catalog, which follows tag merges, and match the sample until one.
		// The mock Playwright suite can add a folder as large as the real
		// library's largest (about 700 files) to check the track tree.
		const big = (globalThis as { __kikoeruMockLargeFolder?: number }).__kikoeruMockLargeFolder ?? 0;
		const extra = Array.from(
			{ length: big },
			(_, i): TrackItem => ({
				...sample.tracks[0]!,
				id: LARGE_FOLDER_FIRST_ID + i,
				path: `扫描图/${String(i + 1).padStart(4, '0')}.jpg`,
				kind: 'image',
				size: 180_000,
				duration_ms: null,
				has_subtitle: false,
				subtitle_formats: [],
				video: null,
				loudness: null
			})
		);
		return {
			...sample,
			tracks: [...sample.tracks, ...extra],
			tags: tagRefs(work),
			...edited,
			favorite: summary.favorite,
			progress,
			my_rating: summary.my_rating,
			my_review: myReview(workId)
		};
	}
	const shift = (workId - SAMPLE_WORK_ID) * TRACK_ID_STRIDE;
	// Other works count as fully probed, with the length their mock WAV has,
	// so their total follows the contract rule instead of always being null.
	const tracks = sample.tracks.map((t) => ({
		...t,
		id: t.id + shift,
		duration_ms:
			t.duration_ms ?? (isPlayable(t) && !t.missing ? MOCK_DEFAULT_TRACK_SECONDS * 1000 : null)
	}));
	return {
		...summary,
		vas: manual?.vas ?? vaRefs(work),
		tags: manual?.tags ?? tagRefs(work),
		total_duration_ms: totalDuration(tracks),
		tracks,
		my_review: myReview(workId),
		locations: [
			{
				root_id: work.root,
				root_name: rootsState.find((r) => r.id === work.root)?.display_name ?? work.root,
				path: summary.code,
				missing: summary.missing
			}
		]
	};
}

const isPlayable = (t: TrackItem) => t.kind === 'audio';

/** Sum of the non-missing audio, or null when one is unprobed or there is none (contract). */
function totalDuration(tracks: TrackItem[]): number | null {
	const audio = tracks.filter((t) => isPlayable(t) && !t.missing);
	if (audio.length === 0 || audio.some((t) => t.duration_ms === null)) return null;
	return audio.reduce((sum, t) => sum + (t.duration_ms ?? 0), 0);
}

function summaryFields({ title, release_date, nsfw, circle }: WorkSummary) {
	return { title, release_date, nsfw, circle };
}

function findTrack(trackId: number): { work: WorkDetail; track: TrackItem } | null {
	// Track ids are the sample's ids shifted by a stride per work.
	const workId = SAMPLE_WORK_ID + Math.round((trackId - (trackId % TRACK_ID_STRIDE)) / TRACK_ID_STRIDE);
	const work = workDetail(workId);
	const track = work?.tracks.find((t) => t.id === trackId);
	return work && track ? { work, track } : null;
}

const tasks = new Map<number, { trackId: number; polls: number }>();
const transcoded = new Set<number>();
let nextTaskId = prepareProcessing.task_id;

// Every mock audio file is a generated WAV (see mock-media.ts), so the mime
// type says so instead of repeating the sample's audio/mpeg.
const MOCK_MIME = 'audio/wav';

/** Where a finished transcode of the track plays from. */
function transcodedMedia(trackId: number, track: TrackItem): { url: string; mime_type: string } {
	if (track.kind === 'video') {
		return { url: `/api/tracks/${trackId}/hls/index.m3u8`, mime_type: prepareReadyHls.mime_type };
	}
	return { url: `/api/tracks/${trackId}/playable`, mime_type: MOCK_MIME };
}

/** WAV stands in for audio that needs the AAC fallback, mkv for video that needs HLS. */
function needsTranscode(track: TrackItem): boolean {
	return track.path.endsWith('.wav') || track.path.endsWith('.mkv');
}

function handlePrepare(trackId: number, body: unknown): ApiResponse {
	const found = findTrack(trackId);
	if (!found) return errorResponse(404, 'not_found', '没有这条轨道');
	const { track } = found;
	if (track.kind !== 'audio' && track.kind !== 'video') {
		return errorResponse(400, 'invalid_request', '只能准备音频和视频');
	}
	if (track.missing) return errorResponse(410, 'missing', '这个文件已经找不到了');
	if (byId.get(found.work.id)?.root === MOCK_DOWN_ROOT.id) return storageDown();
	const forceFallback = (body as Partial<PrepareRequest> | null)?.force_fallback === true;
	// Some tracks stand in for formats that need a transcode, so the waiting
	// state can be seen without a real transcoder.
	if (!forceFallback && !needsTranscode(track)) {
		const mime_type = track.kind === 'video' ? 'video/mp4' : MOCK_MIME;
		return {
			status: 200,
			body: { ...prepareReady, url: `/api/tracks/${trackId}/stream`, mime_type }
		};
	}
	if (transcoded.has(trackId)) {
		return { status: 200, body: { state: 'ready', ...transcodedMedia(trackId, track) } };
	}
	// Like the server, a second prepare of the same track joins the running task.
	const existing = [...tasks].find(([, task]) => task.trackId === trackId);
	const taskId = existing ? existing[0] : nextTaskId++;
	if (!existing) tasks.set(taskId, { trackId, polls: 0 });
	return { status: 202, body: { ...prepareProcessing, task_id: taskId } };
}

function handleTask(taskId: number): ApiResponse {
	const analysis = pollAnalysisTask(taskId);
	if (analysis) return analysis;
	const task = tasks.get(taskId);
	if (!task) {
		const other = getMockTask(taskId);
		return other ? { status: 200, body: other } : errorResponse(404, 'not_found', '没有这个任务');
	}
	task.polls += 1;
	if (task.polls <= MOCK_TASK_RUNNING_POLLS) {
		const progress = task.polls / (MOCK_TASK_RUNNING_POLLS + 1);
		return { status: 200, body: { ...taskRunning, id: taskId, progress } as TaskSnapshot };
	}
	tasks.delete(taskId);
	transcoded.add(task.trackId);
	const found = findTrack(task.trackId);
	if (found?.track.kind === 'video') {
		const result = transcodedMedia(task.trackId, found.track);
		return { status: 200, body: { ...taskDoneHls, id: taskId, result } as TaskSnapshot };
	}
	const result = { url: `/api/tracks/${task.trackId}/playable`, mime_type: MOCK_MIME };
	return { status: 200, body: { ...taskDone, id: taskId, result } as TaskSnapshot };
}

function handleProgress(
	method: string,
	workId: number,
	action: string | undefined,
	body: unknown
): ApiResponse | null {
	const work = workDetail(workId);
	if (!work) return errorResponse(404, 'not_found', '没有这部作品');
	if (method === 'DELETE' && !action) {
		progressOverrides.set(workId, null);
		removeFrom(historyOrder, workId);
		return noContent();
	}
	const { track_id, position_ms, group_path } = (body ?? {}) as Record<string, unknown>;
	const track = work.tracks.find((t) => t.id === track_id);
	if (!track || typeof group_path !== 'string') {
		return errorResponse(400, 'invalid_request', '轨道不属于这部作品');
	}
	const finishedBefore = work.progress?.finished ?? false;
	const base = {
		track_id: track.id,
		track_duration_ms: track.duration_ms,
		resumable: true,
		group_path
	};
	if (method === 'PUT' && !action) {
		if (typeof position_ms !== 'number') return errorResponse(400, 'invalid_request', '缺少位置');
		// Playing a finished work again is a replay (book: data-model.md).
		progressOverrides.set(workId, {
			...base,
			position_ms,
			finished: finishedBefore,
			replaying: finishedBefore
		});
		moveToFront(historyOrder, workId);
		return noContent();
	}
	if (method === 'POST' && action === 'finish') {
		progressOverrides.set(workId, {
			...base,
			position_ms: track.duration_ms ?? 0,
			finished: true,
			replaying: false
		});
		moveToFront(historyOrder, workId);
		return noContent();
	}
	return null;
}

function handleSubtitle(trackId: number, query: ApiRequest['query']): ApiResponse {
	const found = findTrack(trackId);
	if (!found?.track.has_subtitle) return errorResponse(404, 'not_found', '这条轨道没有字幕');
	if (query?.format === 'ass') {
		return found.track.subtitle_formats.includes('ass')
			? { status: 200, body: MOCK_ASS }
			: errorResponse(404, 'not_found', '这条轨道没有 ass 字幕');
	}
	return { status: 200, body: MOCK_SUBTITLE };
}

/** The fonts sample, with ids shifted like the tracks of the work they belong to. */
function handleFonts(trackId: number): ApiResponse {
	const found = findTrack(trackId);
	if (!found) return errorResponse(404, 'not_found', '没有这条轨道');
	const shift = trackId - (trackId % TRACK_ID_STRIDE);
	const fonts: TrackFont[] = fontsSample.map((font) => ({
		...font,
		id: font.id + shift,
		url: `/api/tracks/${font.id + shift}/stream`
	}));
	return { status: 200, body: fonts };
}

function routeM2(request: ApiRequest): ApiResponse | null {
	const { method, path, body } = request;
	let m = /^\/works\/(\d+)$/.exec(path);
	if (m && method === 'GET') {
		const detail = workDetail(Number(m[1]), request.token !== LISTENER_TOKEN);
		return detail ? { status: 200, body: detail } : errorResponse(404, 'not_found', '没有这部作品');
	}
	m = /^\/tracks\/(\d+)\/prepare$/.exec(path);
	if (m && method === 'POST') return handlePrepare(Number(m[1]), body);
	m = /^\/tracks\/(\d+)\/subtitle$/.exec(path);
	if (m && method === 'GET') return handleSubtitle(Number(m[1]), request.query);
	m = /^\/tracks\/(\d+)\/fonts$/.exec(path);
	if (m && method === 'GET') return handleFonts(Number(m[1]));
	m = /^\/tasks\/(\d+)$/.exec(path);
	if (m && method === 'GET') return handleTask(Number(m[1]));
	m = /^\/me\/progress\/(\d+)(?:\/(finish))?$/.exec(path);
	if (m) return handleProgress(method, Number(m[1]), m[2], body);
	m = /^\/me\/favorites\/(\d+)$/.exec(path);
	if (m) return handleFavorite(method, Number(m[1]));
	return null;
}

/**
 * Bodies of the scan requests the mock answered, so the mock Playwright
 * suite can check that the full-scan option reaches the request.
 */
const scanRequests: ScanRequest[] = [];
(globalThis as { __kikoeruMockScans?: ScanRequest[] }).__kikoeruMockScans = scanRequests;

function handleScan(body: unknown): ApiResponse {
	const request = (body ?? {}) as ScanRequest;
	scanRequests.push(structuredClone(request));
	const root = request.root ?? null;
	if (root !== null && !rootsState.some((r) => r.id === root)) {
		return errorResponse(404, 'not_found', '没有这个媒体库');
	}
	const scanned = rootsState.filter((r) => root === null || r.id === root);
	const task_ids = startScans(scanned.map((r) => ({ id: r.id, works: r.work_count })), (id) => {
		const status = rootsState.find((r) => r.id === id);
		if (status) status.last_scan_at = Date.now();
		// A finished scan brings in a tag the samples do not have, so pages
		// can be checked for reloading their tag lists.
		if (!tags.some((t) => t.name === MOCK_SCANNED_TAG)) tags.push({ id: nextTagId++, name: MOCK_SCANNED_TAG, work_count: 1 });
	});
	return { status: 202, body: { ...scanResponse, task_ids } };
}

/** The tag a finished mock scan adds. */
export const MOCK_SCANNED_TAG = '扫描发现的标签';

const DATE = /^\d{4}-\d{2}-\d{2}$/;
const isNameList = (v: unknown): v is string[] => Array.isArray(v) && v.every((x) => typeof x === 'string');

let nextCircleId = 1000;
let nextVaId = 1;
let nextTagId = 1_000_000;

function circleNamed(name: string): CircleRef {
	const found = circles.find((c) => c.name === name);
	if (found) return { id: found.id, name };
	const circle = { id: nextCircleId++, name, work_count: 0 };
	circles.push(circle);
	return { id: circle.id, name };
}

function vaNamed(name: string): VaRef {
	const found = vas.find((v) => v.name === name);
	if (found) return { id: found.id, name };
	const va = { id: `va-mock-${nextVaId++}`, name, work_count: 0 };
	vas.push(va);
	return { id: va.id, name };
}

function tagNamed(name: string): TagRef {
	// A name merged away is an alias of the tag it went into (book/src/scanning.md).
	const aliasOf = admin.aliasOwner(name);
	const found = tags.find((t) => t.name === name || t.id === aliasOf);
	if (found) return { id: found.id, name: found.name };
	const tag = { id: nextTagId++, name, work_count: 0 };
	tags.push(tag);
	return { id: tag.id, name };
}

const byName = (a: { name: string }, b: { name: string }) => a.name.localeCompare(b.name);

function handleMetadata(workId: number, body: unknown): ApiResponse {
	if (!byId.has(workId)) return errorResponse(404, 'not_found', '没有这部作品');
	const r = (body ?? {}) as Partial<MetadataRequest>;
	if (typeof r.title !== 'string' || r.title.trim() === '') {
		return errorResponse(400, 'invalid_request', '标题不能为空');
	}
	if (r.release_date !== null && !(typeof r.release_date === 'string' && DATE.test(r.release_date))) {
		return errorResponse(400, 'invalid_request', '发售日要写成 YYYY-MM-DD');
	}
	if (typeof r.nsfw !== 'boolean' || !isNameList(r.va_names) || !isNameList(r.tag_names)) {
		return errorResponse(400, 'invalid_request', '字段不合法');
	}
	if (r.circle_name !== null && typeof r.circle_name !== 'string') {
		return errorResponse(400, 'invalid_request', '社团名不合法');
	}
	manualMetadata.set(workId, {
		title: r.title,
		release_date: r.release_date,
		nsfw: r.nsfw,
		circle: r.circle_name === null ? null : circleNamed(r.circle_name),
		vas: r.va_names.map(vaNamed).sort(byName),
		tags: r.tag_names.map(tagNamed).sort(byName)
	});
	return { status: 200, body: workDetail(workId) };
}

function handleRefresh(workId: number, query: ApiRequest['query']): ApiResponse {
	if (!byId.has(workId)) return errorResponse(404, 'not_found', '没有这部作品');
	const force = String(query?.force) === 'true';
	// Only a forced refresh replaces manual metadata (book/src/data-model.md).
	const taskId = startRefresh(workId, () => {
		if (force) manualMetadata.delete(workId);
	});
	return { status: 202, body: { ...refreshResponse, task_id: taskId } };
}

function handleCancel(taskId: number): ApiResponse {
	if (tasks.has(taskId)) return errorResponse(400, 'invalid_request', '模拟的转码任务不能取消');
	const result = cancelMockTask(taskId);
	if (result === 'not_found') return errorResponse(404, 'not_found', '没有这个任务');
	if (result === 'ended') return errorResponse(400, 'invalid_request', '任务已经结束');
	return noContent();
}

function handleMoveToFront(taskId: number): ApiResponse {
	if (tasks.has(taskId)) return errorResponse(400, 'invalid_request', '只有扫描任务可以调整顺序');
	const result = moveMockTaskToFront(taskId);
	if (result === 'not_found') return errorResponse(404, 'not_found', '没有这个任务');
	if (result === 'not_queued_scan') return errorResponse(400, 'invalid_request', '扫描已经开始或已经结束');
	return noContent();
}

function handleCleanup(body: unknown): ApiResponse {
	if (scanRunning()) {
		return errorResponse(409, 'scan_running', '有扫描正在进行，等它结束之后再清理');
	}
	const result = admin.cleanup(body as CleanupRequest);
	for (const id of (body as CleanupRequest).work_ids) {
		if (!catalog.some((w) => w.summary.id === id)) byId.delete(id);
	}
	return { status: 200, body: result };
}

function handleMerge(body: unknown): ApiResponse {
	const request = body as TagMergeRequest;
	const result = admin.mergeTags(request);
	if (!result.ok) return errorResponse(400, 'invalid_request', result.message);
	// Manual metadata keeps its own tag list, which the merge moves as well.
	const into = tags.find((t) => t.id === request.into);
	for (const manual of manualMetadata.values()) {
		if (!into || !manual.tags.some((t) => t.id === request.from)) continue;
		const kept = manual.tags.filter((t) => t.id !== request.from && t.id !== into.id);
		manual.tags = [...kept, { id: into.id, name: into.name }].sort(byName);
	}
	return noContent();
}

const ADMIN_ONLY = [
	/^\/admin\//,
	/^\/tasks$/,
	/^\/tasks\/\d+\/(cancel|move-to-front|events)$/,
	/^\/works\/\d+\/(metadata|refresh)$/
];

function isAdminOnly({ path }: { path: string }): boolean {
	return ADMIN_ONLY.some((re) => re.test(path));
}

function routeM4(request: ApiRequest): ApiResponse | null {
	const { method, path, body, query } = request;
	const key = `${method} ${path}`;
	switch (key) {
		case 'POST /admin/scan':
			return handleScan(body);
		case 'GET /tasks': {
			const page = listMockTasks(query);
			return page ? { status: 200, body: page } : errorResponse(400, 'invalid_request', '不认识的任务状态或种类');
		}
		case 'GET /admin/missing':
			return { status: 200, body: admin.getMissing() };
		case 'POST /admin/missing/cleanup':
			return handleCleanup(body);
		case 'POST /admin/tags/merge':
			return handleMerge(body);
	}
	const root = routeRoots(request);
	if (root) return root;
	let m = /^\/tasks\/(\d+)\/cancel$/.exec(path);
	if (m && method === 'POST') return handleCancel(Number(m[1]));
	m = /^\/tasks\/(\d+)\/move-to-front$/.exec(path);
	if (m && method === 'POST') return handleMoveToFront(Number(m[1]));
	m = /^\/works\/(\d+)\/metadata$/.exec(path);
	if (m && method === 'PUT') return handleMetadata(Number(m[1]), body);
	m = /^\/works\/(\d+)\/refresh$/.exec(path);
	if (m && method === 'POST') return handleRefresh(Number(m[1]), query);
	return null;
}

/** Media tokens issued since the page loaded, so each refresh gets a new one. */
let mediaTokenCount = 0;

function route(request: ApiRequest): ApiResponse {
	const { method, path, token } = request;
	const key = `${method} ${path}`;

	switch (key) {
		case 'GET /health':
			return { status: 200, body: { status: 'ok' } };
		case 'GET /ready':
			return { status: 200, body: { status: 'ready' } };
		case 'GET /version':
			return { status: 200, body: versionResponse };
		case 'POST /auth/login':
			return handleLogin(request.body);
	}

	if (!token) return { status: 401, body: errorUnauthorized };

	switch (key) {
		case 'POST /auth/logout-all':
			return { status: 204, body: null };
		case 'GET /auth/me':
			return { status: 200, body: token === LISTENER_TOKEN ? LISTENER : meResponse };
		case 'POST /auth/media-token':
			return {
				status: 200,
				body: { ...mediaTokenResponse, token: mockMediaToken(mediaTokenResponse.token, mediaTokenCount++) }
			};
		case 'PUT /auth/password':
			return handlePasswordChange(request.body);
		case 'GET /works':
			return handleWorks(request.query);
		case 'GET /me/favorites':
			return handleUserList(favoriteOrder, request.query);
		case 'GET /me/history':
			return handleUserList(historyOrder, request.query);
		case 'GET /tags':
			return { status: 200, body: tags };
		case 'GET /circles':
			return { status: 200, body: circles };
		case 'GET /vas':
			return { status: 200, body: vas };
	}
	if (token === LISTENER_TOKEN && isAdminOnly(request)) {
		return errorResponse(403, 'forbidden', '需要管理员权限');
	}
	return (
		routeM2(request) ??
		routeAnalysis(request, (id) => findTrack(id)?.track ?? null) ??
		routeReviews(request, (id) => {
			const work = byId.get(id);
			return work ? currentSummary(work) : null;
		}) ??
		routeM4(request) ??
		routeSettings(request) ??
		errorResponse(404, 'not_found', `模拟数据里没有 ${key}`)
	);
}

function delay(): Promise<void> {
	// The mock Playwright suite sets a fixed latency to widen race windows.
	const fixed = (globalThis as { __kikoeruMockLatencyMs?: number }).__kikoeruMockLatencyMs;
	const ms = fixed ?? MIN_LATENCY_MS + Math.random() * (MAX_LATENCY_MS - MIN_LATENCY_MS);
	return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Requests the mock answered, keyed by `METHOD /path`, so the mock
 * Playwright suite can assert how often a page asks for something.
 */
const requestCounts: Record<string, number> = {};
(globalThis as { __kikoeruMockRequests?: Record<string, number> }).__kikoeruMockRequests =
	requestCounts;

export const mockTransport: Transport = async (request) => {
	const key = `${request.method} ${request.path}`;
	requestCounts[key] = (requestCounts[key] ?? 0) + 1;
	await delay();
	// The mock Playwright suite can keep a request from reaching the mock until
	// it lets it go, so a save stays in flight exactly as long as a test needs
	// instead of for a fixed time that a slow machine can outrun.
	const gate = (globalThis as { __kikoeruMockGates?: Record<string, { promise: Promise<void> }> })
		.__kikoeruMockGates?.[key];
	if (gate) await gate.promise;
	// Clone so a page mutating a response cannot change the next one.
	// The mock Playwright suite can make the next request to one key fail.
	const hooks = globalThis as { __kikoeruMockFailOnce?: string | null };
	const failing = hooks.__kikoeruMockFailOnce === key;
	if (failing) hooks.__kikoeruMockFailOnce = null;
	const response = failing
		? errorResponse(500, 'internal_error', '模拟的服务器错误')
		: structuredClone(route(request));
	// The mock Playwright suite can hold one answer back after it was
	// computed, like a slow network after the server has replied.
	const held = (globalThis as { __kikoeruMockHoldMs?: Record<string, number> }).__kikoeruMockHoldMs?.[key];
	if (held) await new Promise((resolve) => setTimeout(resolve, held));
	// Or until it lets the answers to one key go, when a fixed time could run
	// out early on a slow machine.
	const waiting = (globalThis as { __kikoeruMockWait?: Record<string, Promise<void>> }).__kikoeruMockWait?.[key];
	if (waiting) await waiting;
	return response;
};

/** How often the mock event stream looks for a change, like the server's poll of the task row. */
const MOCK_EVENT_POLL_MS = 250;
const MOCK_PING_MS = 15_000;

function jsonResponse({ status, body }: ApiResponse): Response {
	return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
}

/**
 * Answers GET /tasks/{id}/events with a real event stream, so the page reads
 * it through the same parser as against the server: a snapshot first, one
 * per change, the terminal one last, then the end of the stream.
 */
export const mockStreamTransport: StreamTransport = async ({ path, token, signal }) => {
	const key = `GET ${path}`;
	requestCounts[key] = (requestCounts[key] ?? 0) + 1;
	await delay();
	if (!token) return jsonResponse({ status: 401, body: errorUnauthorized });
	// The mock Playwright suite can make the next stream whose key starts
	// with this text answer 404, as for a task that is gone.
	const hooks = globalThis as { __kikoeruMockStreamGoneOnce?: string | null };
	if (hooks.__kikoeruMockStreamGoneOnce && key.startsWith(hooks.__kikoeruMockStreamGoneOnce)) {
		hooks.__kikoeruMockStreamGoneOnce = null;
		return jsonResponse(errorResponse(404, 'not_found', '没有这个任务'));
	}
	if (token === LISTENER_TOKEN) return jsonResponse(errorResponse(403, 'forbidden', '需要管理员权限'));
	const m = /^\/tasks\/(\d+)\/events$/.exec(path);
	const taskId = m ? Number(m[1]) : NaN;
	const first = m ? getMockTask(taskId) : null;
	if (!first) return jsonResponse(errorResponse(404, 'not_found', '没有这个任务'));

	const encoder = new TextEncoder();
	let timer: ReturnType<typeof setInterval> | undefined;
	const body = new ReadableStream<Uint8Array>({
		start(controller) {
			let last = '';
			let quiet = 0;
			const stop = () => {
				clearInterval(timer);
				signal.removeEventListener('abort', stop);
			};
			const tick = () => {
				const snapshot = getMockTask(taskId) as TaskSnapshot;
				const text = JSON.stringify(snapshot);
				if (text !== last) {
					last = text;
					quiet = 0;
					controller.enqueue(encoder.encode(`event: snapshot\ndata: ${text}\n\n`));
				} else if ((quiet += MOCK_EVENT_POLL_MS) >= MOCK_PING_MS) {
					quiet = 0;
					controller.enqueue(encoder.encode(': ping\n\n'));
				}
				if (isTerminalState(snapshot.state)) {
					stop();
					controller.close();
				}
			};
			signal.addEventListener('abort', stop);
			tick();
			if (!isTerminalState(first.state)) timer = setInterval(tick, MOCK_EVENT_POLL_MS);
		},
		cancel() {
			clearInterval(timer);
		}
	});
	return new Response(body, { status: 200, headers: { 'Content-Type': 'text/event-stream' } });
};
