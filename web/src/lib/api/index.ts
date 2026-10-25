import { openStream, request } from './client';
import type {
	CircleCount,
	HealthResponse,
	LoginRequest,
	LoginResponse,
	MediaTokenResponse,
	Page,
	PasswordRequest,
	ReadyResponse,
	User,
	VersionResponse,
	WorkSummary,
	FinishRequest,
	ProgressRequest,
	RootCreateRequest,
	RootStatus,
	RootUpdateRequest,
	ScanRequest,
	TagCount,
	VaCount,
	ScanResponse,
	TaskSnapshot,
	TrackFont,
	WorkDetail,
	CleanupRequest,
	CleanupResponse,
	MetadataRequest,
	MissingReport,
	RefreshResponse,
	TagMergeRequest
} from './types';
import type { ProxyCheckResult, ServerSettings } from './types';
import type { PageQuery, TasksQuery, WorksQuery } from './query';
import type { TaskWatchDeps } from '../tasks/task-watch';
import type { Loudness, PrepareRequest, PrepareResponse, UserSettings } from './types';
import type { Review, ReviewInput } from './types';

export { ApiError } from './errors';
export { setTokenSource, setUnauthorizedHandler } from './client';
export { session } from '../stores/session.svelte';

export const DEFAULT_PAGE_SIZE = 40;

export function health(): Promise<HealthResponse> {
	return request({ method: 'GET', path: '/health', auth: false });
}

export function ready(): Promise<ReadyResponse> {
	return request({ method: 'GET', path: '/ready', auth: false });
}

export function version(): Promise<VersionResponse> {
	return request({ method: 'GET', path: '/version', auth: false });
}

export function login(body: LoginRequest): Promise<LoginResponse> {
	return request({ method: 'POST', path: '/auth/login', body, auth: false });
}

export function me(): Promise<User> {
	return request({ method: 'GET', path: '/auth/me' });
}

export function mediaToken(): Promise<MediaTokenResponse> {
	return request({ method: 'POST', path: '/auth/media-token' });
}

/** Resolves on 204. Every token of the user is revoked afterwards. */
export async function changePassword(body: PasswordRequest): Promise<void> {
	await request<null>({ method: 'PUT', path: '/auth/password', body });
}

export function listWorks(query: WorksQuery = {}): Promise<Page<WorkSummary>> {
	return request({
		method: 'GET',
		path: '/works',
		query: { page_size: DEFAULT_PAGE_SIZE, ...query }
	});
}

export function getWork(id: number): Promise<WorkDetail> {
	return request({ method: 'GET', path: `/works/${id}` });
}

/** Resolves for both the 200 `ready` and the 202 `processing` answer. */
export function prepareTrack(trackId: number, body: PrepareRequest): Promise<PrepareResponse> {
	return request({ method: 'POST', path: `/tracks/${trackId}/prepare`, body });
}

export function getTask(taskId: number): Promise<TaskSnapshot> {
	return request({ method: 'GET', path: `/tasks/${taskId}` });
}

/** WebVTT text of the subtitle that belongs to an audio or video track. */
export function getSubtitle(trackId: number): Promise<string> {
	return request({ method: 'GET', path: `/tracks/${trackId}/subtitle`, responseType: 'text' });
}

/** The raw ass subtitle of a track whose `subtitle_formats` lists `ass`, as UTF-8 text. */
export function getAssSubtitle(trackId: number): Promise<string> {
	return request({
		method: 'GET',
		path: `/tracks/${trackId}/subtitle`,
		query: { format: 'ass' },
		responseType: 'text'
	});
}

/** Font files of the track's work, for rendering its ass subtitle. */
export function listTrackFonts(trackId: number): Promise<TrackFont[]> {
	return request({ method: 'GET', path: `/tracks/${trackId}/fonts` });
}

/** The 202 answer of the loudness and waveform endpoints while an analysis runs. */
export type AnalysisPending = Extract<PrepareResponse, { state: 'processing' }>;

/** Resolves for the 200 measurement and for the 202 `processing` answer. */
export function getLoudness(trackId: number): Promise<Loudness | AnalysisPending> {
	return request({ method: 'GET', path: `/tracks/${trackId}/loudness` });
}

/**
 * The 1000 peaks of a track, or the 202 `processing` answer. The browser's
 * HTTP cache keeps the body for a week and revalidates it by its ETag.
 */
export function getWaveform(trackId: number): Promise<Uint8Array | AnalysisPending> {
	return request({ method: 'GET', path: `/tracks/${trackId}/waveform`, responseType: 'bytes' });
}

export function getUserSettings(): Promise<UserSettings> {
	return request({ method: 'GET', path: '/me/settings' });
}

/** Replaces all settings and answers what was saved. */
export function putUserSettings(body: UserSettings): Promise<UserSettings> {
	return request({ method: 'PUT', path: '/me/settings', body });
}

export async function putProgress(
	workId: number,
	body: ProgressRequest,
	options: { keepalive?: boolean } = {}
): Promise<void> {
	await request<null>({ method: 'PUT', path: `/me/progress/${workId}`, body, ...options });
}

export async function finishWork(workId: number, body: FinishRequest): Promise<void> {
	await request<null>({ method: 'POST', path: `/me/progress/${workId}/finish`, body });
}

export async function clearProgress(workId: number): Promise<void> {
	await request<null>({ method: 'DELETE', path: `/me/progress/${workId}` });
}

export function listRoots(): Promise<RootStatus[]> {
	return request({ method: 'GET', path: '/admin/roots' });
}

/** Answers 201 with the new root, which has not been scanned yet. */
export function createRoot(body: RootCreateRequest): Promise<RootStatus> {
	return request({ method: 'POST', path: '/admin/roots', body });
}

/** Replaces the display name and settings. A null password keeps the stored one. */
export function updateRoot(id: string, body: RootUpdateRequest): Promise<RootStatus> {
	return request({ method: 'PUT', path: `/admin/roots/${encodeURIComponent(id)}`, body });
}

/** Resolves on 204. The root's locations are marked missing, see `getMissing`. */
export async function deleteRoot(id: string): Promise<void> {
	await request<null>({ method: 'DELETE', path: `/admin/roots/${encodeURIComponent(id)}` });
}

export function startScan(body: ScanRequest): Promise<ScanResponse> {
	return request({ method: 'POST', path: '/admin/scan', body });
}

export function getServerSettings(): Promise<ServerSettings> {
	return request({ method: 'GET', path: '/admin/settings' });
}

/** Replaces every server setting and answers them as saved, in normalised form. */
export function putServerSettings(body: ServerSettings): Promise<ServerSettings> {
	return request({ method: 'PUT', path: '/admin/settings', body });
}

/**
 * Asks DLsite through the proxy in `body` without saving it. Any HTTP answer
 * resolves, a 403 included. No answer at all rejects with 502 `upstream_error`.
 */
export function checkProxy(body: ServerSettings): Promise<ProxyCheckResult> {
	return request({ method: 'POST', path: '/admin/settings/proxy-check', body });
}

/** Favorites of the current user, newest addition first. */
export function listFavorites(query: PageQuery = {}): Promise<Page<WorkSummary>> {
	return request({
		method: 'GET',
		path: '/me/favorites',
		query: { page_size: DEFAULT_PAGE_SIZE, ...query }
	});
}

/** Works with saved progress, most recently played first. */
export function listHistory(query: PageQuery = {}): Promise<Page<WorkSummary>> {
	return request({
		method: 'GET',
		path: '/me/history',
		query: { page_size: DEFAULT_PAGE_SIZE, ...query }
	});
}

/** Resolves on 204. Repeating it has no further effect. */
export async function addFavorite(workId: number): Promise<void> {
	await request<null>({ method: 'PUT', path: `/me/favorites/${workId}` });
}

/** Revokes every API and media token of the user, this one included. */
export async function logoutAll(): Promise<void> {
	await request<null>({ method: 'POST', path: '/auth/logout-all' });
}

export async function removeFavorite(workId: number): Promise<void> {
	await request<null>({ method: 'DELETE', path: `/me/favorites/${workId}` });
}

/** Works the current user rated or reviewed, most recently changed first. */
export function listReviews(query: PageQuery = {}): Promise<Page<WorkSummary>> {
	return request({
		method: 'GET',
		path: '/me/reviews',
		query: { page_size: DEFAULT_PAGE_SIZE, ...query }
	});
}

/** Replaces the rating and text. Clearing both is a `deleteReview`, which the server requires. */
export function putReview(workId: number, body: ReviewInput): Promise<Review> {
	return request({ method: 'PUT', path: `/me/reviews/${workId}`, body });
}

/** Resolves on 204, also when the work had no review. */
export async function deleteReview(workId: number): Promise<void> {
	await request<null>({ method: 'DELETE', path: `/me/reviews/${workId}` });
}

export function listTags(): Promise<TagCount[]> {
	return request({ method: 'GET', path: '/tags' });
}

export function listCircles(): Promise<CircleCount[]> {
	return request({ method: 'GET', path: '/circles' });
}

export function listVas(): Promise<VaCount[]> {
	return request({ method: 'GET', path: '/vas' });
}

/** Every task, newest first. Admins only. */
export function listTasks(query: TasksQuery = {}): Promise<Page<TaskSnapshot>> {
	return request({ method: 'GET', path: '/tasks', query: { ...query } });
}

/** Resolves on 204. The task stops when it is safe and ends as `cancelled`. */
export async function cancelTask(taskId: number): Promise<void> {
	await request<null>({ method: 'POST', path: `/tasks/${taskId}/cancel` });
}

/**
 * Resolves on 204. A queued scan becomes the next one to start, after the
 * scan that is running now.
 */
export async function moveTaskToFront(taskId: number): Promise<void> {
	await request<null>({ method: 'POST', path: `/tasks/${taskId}/move-to-front` });
}

/** How the task watcher reaches the server: the SSE stream, and the snapshot to poll. */
export const taskWatchDeps: TaskWatchDeps = {
	open: (taskId, signal) => openStream(`/tasks/${taskId}/events`, signal),
	poll: getTask
};

export function getMissing(): Promise<MissingReport> {
	return request({ method: 'GET', path: '/admin/missing' });
}

export function cleanupMissing(body: CleanupRequest): Promise<CleanupResponse> {
	return request({ method: 'POST', path: '/admin/missing/cleanup', body });
}

/** Resolves on 204 once `from` is merged into `into` and deleted. */
export async function mergeTags(body: TagMergeRequest): Promise<void> {
	await request<null>({ method: 'POST', path: '/admin/tags/merge', body });
}

/** Replaces the work's metadata and marks it manual. Answers the updated detail. */
export function putMetadata(workId: number, body: MetadataRequest): Promise<WorkDetail> {
	return request({ method: 'PUT', path: `/works/${workId}/metadata`, body });
}

/**
 * Fetches the work's metadata and cover again. Without `force`, manual
 * metadata and imported or manual covers stay as they are.
 */
export function refreshWork(workId: number, force: boolean): Promise<RefreshResponse> {
	return request({
		method: 'POST',
		path: `/works/${workId}/refresh`,
		query: force ? { force: true } : undefined
	});
}
