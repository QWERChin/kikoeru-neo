// Shared between the config, the global setup and the tests. The ports are
// fixed per run through the environment so every worker sees the same ones.
export const E2E_PORT = Number(process.env.KIKOERU_E2E_PORT ?? 18_931);
export const E2E_BASE_URL = `http://127.0.0.1:${E2E_PORT}`;

/**
 * The playback suite runs on a second server, so the works its scan adds
 * never show up in the library pages the first suite counts.
 */
export const PLAYBACK_PORT = Number(process.env.KIKOERU_E2E_PLAYBACK_PORT ?? E2E_PORT + 1);
export const PLAYBACK_BASE_URL = `http://127.0.0.1:${PLAYBACK_PORT}`;

/**
 * The metadata suite runs on a third server that scrapes a fake DLsite, so
 * the fetched titles never replace the synthetic ones the other suites check.
 */
export const METADATA_PORT = Number(process.env.KIKOERU_E2E_METADATA_PORT ?? E2E_PORT + 2);
export const METADATA_BASE_URL = `http://127.0.0.1:${METADATA_PORT}`;

export const ADMIN = { name: 'e2e-admin', password: 'e2e admin password' };
export const LISTENER = { name: 'e2e-listener', password: 'e2e listener password' };

/**
 * One listener per browser project on the playback server. Progress is per
 * user, so the desktop and phone runs never see each other's positions.
 */
export function playbackUser(project: string) {
	return { name: `e2e-play-${project}`, password: `e2e play ${project} password` };
}
export const PLAYBACK_PROJECTS = ['desktop', 'phone', 'insecure', 'insecure-firefox'] as const;

/** Set by the global setup: the playback server's data directory. */
export const PLAYBACK_DATA_ENV = 'KIKOERU_E2E_PLAYBACK_DATA';
/** Set by the global setup: the directory that holds the metadata server's roots. */
export const METADATA_ROOTS_ENV = 'KIKOERU_E2E_METADATA_ROOTS';
/** Set by the global setup: where the fake DLsite and the fake OpenList listen. */
export const DLSITE_URL_ENV = 'KIKOERU_E2E_DLSITE_URL';
export const OPENLIST_URL_ENV = 'KIKOERU_E2E_OPENLIST_URL';
/** Set by the global setup: the fake WebDAV share's URL, and the directory it serves. */
export const WEBDAV_URL_ENV = 'KIKOERU_E2E_WEBDAV_URL';
export const WEBDAV_SHARE_ENV = 'KIKOERU_E2E_WEBDAV_SHARE';

/** Reads a variable the global setup exported to the workers. */
export function setupEnv(name: string): string {
	const value = process.env[name];
	if (!value) throw new Error(`${name} is not set; the global setup did not run`);
	return value;
}

/** Synthetic library: enough works for two pages of 40. */
export const WORK_COUNT = 45;
export const PAGE_SIZE = 40;

/**
 * The insecure projects open the playback server under an origin the
 * browser does not count as secure, the way a LAN address over plain HTTP
 * is. Browsers trust localhost and all of 127.0.0.0/8, so by default the
 * page uses a host name nothing resolves, and the browser sends every
 * request for it to the playback server as its HTTP proxy. The secure
 * context check reads the URL, not the socket. With `KIKOERU_E2E_LAN_IP`
 * set, the playback server also listens on every interface and the page
 * uses that real address directly.
 */
export const LAN_IP = process.env.KIKOERU_E2E_LAN_IP || null;
export const INSECURE_HOST = LAN_IP ?? 'kikoeru-lan.test';
export const INSECURE_BASE_URL = `http://${INSECURE_HOST}:${PLAYBACK_PORT}`;
/**
 * Control run only: the browser is told to treat the insecure origin as
 * secure, so the suite can show its first check really tells the two apart.
 */
export const INSECURE_CONTROL = !!process.env.KIKOERU_E2E_INSECURE_CONTROL;
