import { request } from './client';
import { ApiError } from './errors';
import { session } from '../stores/session.svelte';
import type { MediaTokenResponse, WorkSummary } from './types';

let refreshing: { sessionKey: string | null; promise: Promise<string> } | null = null;

/**
 * Exchanges the API token for a new media token. Concurrent callers of the
 * same session share one request. A refresh started by another session is
 * never shared, and its answer is rejected rather than returned if the
 * session changed meanwhile, so one account's media token can never end up
 * in another account's media URLs.
 */
export function refreshMediaToken(): Promise<string> {
	// The account, not the token: a token the desktop shell renews during
	// the request is still the same session.
	const sessionKey = session.key;
	if (refreshing && refreshing.sessionKey === sessionKey) return refreshing.promise;
	const promise = request<MediaTokenResponse>({ method: 'POST', path: '/auth/media-token' })
		.then((response) => {
			if (session.key !== sessionKey) {
				throw new ApiError(401, 'unauthorized', '登录状态已变化，请重试');
			}
			session.setMediaToken(response.token);
			return response.token;
		})
		.finally(() => {
			if (refreshing?.promise === promise) refreshing = null;
		});
	refreshing = { sessionKey, promise };
	return promise;
}

export async function ensureMediaToken(): Promise<string> {
	return session.mediaToken ?? refreshMediaToken();
}

/** Adds the media token to a relative media URL such as `/api/tracks/1/stream`. */
export function withMediaToken(url: string, token: string | null): string {
	if (!token) return url;
	const separator = url.includes('?') ? '&' : '?';
	return `${url}${separator}token=${encodeURIComponent(token)}`;
}

export type CoverSize = 'thumb' | 'full';

/**
 * A cover's URL. Pages usually leave `token` null and rely on the media
 * token cookie, so the URL, and the browser's cache entry, survive a token
 * renewal (see `coverToken`). The media session's artwork is fetched
 * outside the page, so it passes the token.
 */
export function coverUrl(
	work: Pick<WorkSummary, 'id' | 'cover_version'>,
	size: CoverSize,
	token: string | null
): string {
	const params = new URLSearchParams();
	if (size === 'thumb') params.set('size', 'thumb');
	// The version only busts caches when the cover changes; the server ignores it.
	params.set('v', String(work.cover_version));
	return withMediaToken(`/api/works/${work.id}/cover?${params}`, token);
}
