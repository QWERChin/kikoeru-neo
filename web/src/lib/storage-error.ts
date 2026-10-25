import { ApiError } from './api/errors';
import type { RootStatus } from './api/types';

/**
 * A 503 storage_unavailable means a media library cannot be reached right
 * now. The works, favorites and progress in the database are untouched, so
 * pages say that instead of treating the works as gone (book/src/api.md).
 */
export function isStorageUnavailable(error: unknown): error is ApiError {
	return error instanceof ApiError && error.code === 'storage_unavailable';
}

export const STORAGE_REASSURANCE = '作品、收藏和播放进度都还在，媒体库恢复之后重试即可。';

/**
 * The sentence that names the unreachable library. The error body has no
 * root field, so the name comes from what the page knows: the library the
 * user filtered by, then the libraries an admin's root list reports as
 * unavailable, and otherwise the server's own message, which names the
 * library when the server knows it.
 */
export function storageUnavailableText(
	error: ApiError,
	context: { rootName?: string | null; roots?: readonly RootStatus[] | null } = {}
): string {
	if (context.rootName) return `媒体库“${context.rootName}”暂时无法访问。`;
	const down = (context.roots ?? []).filter((r) => r.status === 'unavailable');
	if (down.length > 0) {
		return `媒体库${down.map((r) => `“${r.display_name}”`).join('、')}暂时无法访问。`;
	}
	const message = error.message.trim();
	if (message === '') return '媒体库暂时无法访问。';
	return /[。.！!]$/.test(message) ? message : `${message}。`;
}
