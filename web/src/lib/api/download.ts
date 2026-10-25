import { refreshMediaToken, withMediaToken } from './media';

/** A track's original file, saved under its own name. */
export function trackDownloadPath(trackId: number): string {
	return `/api/tracks/${trackId}/download`;
}

/** A zip of the whole work, or of one folder and everything below it. */
export function workDownloadPath(workId: number, folder: string | null): string {
	const path = `/api/works/${workId}/download`;
	return folder ? `${path}?${new URLSearchParams({ group: folder })}` : path;
}

/**
 * Hands a download URL to the browser. A link cannot send the Authorization
 * header, so the URL carries a media token. It is always a fresh one: the
 * browser's download manager cannot retry a 401 the way the player does, and
 * the stored token may have expired while the page stayed open.
 */
export async function startDownload(path: string): Promise<void> {
	const token = await refreshMediaToken();
	const link = document.createElement('a');
	link.href = withMediaToken(path, token);
	// Empty, so the file name comes from the server's Content-Disposition.
	link.download = '';
	link.click();
}
