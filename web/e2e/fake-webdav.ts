import { createReadStream, readdirSync, statSync, type Stats } from 'node:fs';
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';
import { join } from 'node:path';
import { WEBDAV_URL_ENV, setupEnv } from './env';

/**
 * A stand-in for a WebDAV share that serves a local directory below
 * `/dav`: Basic auth, `PROPFIND` with `Depth: 0` and `1`, and `GET` with
 * Range. It answers like the fake in crates/webdav-client/tests/common/mod.rs,
 * but reads real files so the tests can copy works in while it runs.
 */
export const WEBDAV_USER = 'e2e-dav';
export const WEBDAV_PASSWORD = 'e2e synthetic webdav password';
const PREFIX = '/dav';
const AUTHORIZATION = `Basic ${Buffer.from(`${WEBDAV_USER}:${WEBDAV_PASSWORD}`).toString('base64')}`;
// A fixed time, so listings of an unchanged tree are identical.
const MODIFIED = new Date(1_791_700_000_000).toUTCString();

export interface WebDavState {
	/** Every request as "METHOD decoded-path depth", Range for GET. */
	requests: string[];
}

export interface FakeWebDav {
	/** The share's root, which a root's `url` points at. */
	url: string;
	close(): Promise<void>;
}

const escapeXml = (text: string) => text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
const encodePath = (path: string) => path.split('/').map(encodeURIComponent).join('/');

function entry(path: string, stats: Stats): string {
	const props = stats.isDirectory()
		? '<D:resourcetype><D:collection/></D:resourcetype>'
		: `<D:resourcetype/><D:getcontentlength>${stats.size}</D:getcontentlength>`;
	const href = `${PREFIX}${encodePath(path)}${stats.isDirectory() ? '/' : ''}`;
	return (
		`<D:response><D:href>${escapeXml(href)}</D:href><D:propstat><D:prop>${props}` +
		`<D:getetag>"${stats.size}-${stats.mtimeMs}"</D:getetag><D:getlastmodified>${MODIFIED}</D:getlastmodified>` +
		'</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>'
	);
}

export async function startFakeWebDav(dir: string): Promise<FakeWebDav> {
	const state: WebDavState = { requests: [] };
	const local = (path: string) => join(dir, ...path.split('/').filter(Boolean));
	const stat = (path: string) => {
		try {
			return statSync(local(path));
		} catch {
			return null;
		}
	};

	function propfind(response: ServerResponse, path: string, depth: string) {
		const stats = stat(path);
		if (!stats) {
			response.writeHead(404).end();
			return;
		}
		let body = '<?xml version="1.0" encoding="utf-8"?><D:multistatus xmlns:D="DAV:">';
		body += entry(path, stats);
		if (depth === '1' && stats.isDirectory()) {
			for (const name of readdirSync(local(path)).sort()) {
				const child = `${path}/${name}`;
				body += entry(child, statSync(local(child)));
			}
		}
		body += '</D:multistatus>';
		response.writeHead(207, { 'content-type': 'application/xml; charset=utf-8' });
		response.end(body);
	}

	function get(request: IncomingMessage, response: ServerResponse, path: string) {
		const stats = stat(path);
		if (!stats?.isFile()) {
			response.writeHead(404).end();
			return;
		}
		const size = stats.size;
		const match = /^bytes=(\d+)-(\d*)$/.exec(request.headers.range ?? '');
		if (!match) {
			response.writeHead(200, { 'content-length': String(size) });
			createReadStream(local(path)).pipe(response);
			return;
		}
		const start = Number(match[1]);
		if (start >= size) {
			response.writeHead(416, { 'content-range': `bytes */${size}` }).end();
			return;
		}
		const end = Math.min(match[2] ? Number(match[2]) : size - 1, size - 1);
		response.writeHead(206, { 'content-range': `bytes ${start}-${end}/${size}`, 'content-length': String(end - start + 1) });
		createReadStream(local(path), { start, end }).pipe(response);
	}

	const server = createServer((request, response) => {
		const url = new URL(request.url ?? '/', 'http://fake');
		if (url.pathname === '/__e2e/state') {
			response.writeHead(200, { 'content-type': 'application/json' });
			response.end(JSON.stringify(state));
			return;
		}
		if (url.pathname !== PREFIX && !url.pathname.startsWith(`${PREFIX}/`)) {
			response.writeHead(404).end();
			return;
		}
		const path = decodeURIComponent(url.pathname.slice(PREFIX.length)).replace(/\/+$/, '');
		const depth = String(request.headers.depth ?? '-');
		state.requests.push(`${request.method} ${path} ${request.method === 'GET' ? (request.headers.range ?? '-') : depth}`);
		// The body of a PROPFIND only names properties, and every answer carries the same ones.
		request.resume();
		if (request.headers.authorization !== AUTHORIZATION) {
			response.writeHead(401, { 'www-authenticate': 'Basic realm="e2e"' }).end();
			return;
		}
		if (request.method === 'PROPFIND') propfind(response, path, depth);
		else if (request.method === 'GET') get(request, response, path);
		else response.writeHead(405).end();
	});
	await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
	const port = (server.address() as AddressInfo).port;
	return {
		url: `http://127.0.0.1:${port}${PREFIX}`,
		close: async () => {
			server.closeAllConnections();
			await new Promise((resolve) => server.close(resolve));
		}
	};
}

/** The fake as the tests see it, from the URL the global setup exported. */
export const webdav = {
	url: () => setupEnv(WEBDAV_URL_ENV),
	async state(): Promise<WebDavState> {
		const base = new URL(setupEnv(WEBDAV_URL_ENV));
		return (await (await fetch(new URL('/__e2e/state', base))).json()) as WebDavState;
	}
};
