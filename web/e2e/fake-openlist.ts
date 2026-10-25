import { createReadStream, readdirSync, statSync, type Stats } from 'node:fs';
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';
import { join } from 'node:path';
import { OPENLIST_URL_ENV, setupEnv } from './env';

/**
 * A stand-in for OpenList that serves a local directory: login, listing,
 * file info, links and the files themselves with Range. It answers like the
 * fake in crates/neo-server/tests/process_m6.rs.
 *
 * `/__e2e/hold` makes whole-file downloads of one path wait until it is
 * cleared. A transcode copies its remote source with such a download, while
 * the probe before it asks for a Range, so holding the path parks the
 * transcode mid-generation for as long as a test needs.
 */
export const OPENLIST_USER = 'e2e-reader';
export const OPENLIST_PASSWORD = 'e2e synthetic openlist password';

export interface OpenListState {
	hold: string | null;
	/** Downloads of the held path waiting right now. */
	held: number;
	downloads: { path: string; range: string | null }[];
}

async function body(request: IncomingMessage): Promise<Record<string, unknown>> {
	let text = '';
	for await (const chunk of request) text += chunk;
	return text ? (JSON.parse(text) as Record<string, unknown>) : {};
}

function entry(name: string, stats: Stats) {
	return {
		name,
		size: stats.isDirectory() ? 0 : stats.size,
		is_dir: stats.isDirectory(),
		modified: '2024-05-01T20:00:00+08:00',
		created: '0001-01-01T00:00:00Z',
		sign: '',
		thumb: '',
		type: 0,
		hashinfo: 'null',
		hash_info: null
	};
}

export interface FakeOpenList {
	url: string;
	close(): Promise<void>;
}

export async function startFakeOpenList(dir: string): Promise<FakeOpenList> {
	const state: OpenListState = { hold: null, held: 0, downloads: [] };
	let release: (() => void)[] = [];
	let base = '';
	const local = (path: string) => join(dir, ...path.split('/').filter(Boolean));
	const stat = (path: string) => {
		try {
			return statSync(local(path));
		} catch {
			return null;
		}
	};
	const reply = (response: ServerResponse, data: unknown, code = 200, message = 'success') => {
		response.writeHead(200, { 'content-type': 'application/json' });
		response.end(JSON.stringify({ code, message, data }));
	};

	async function file(request: IncomingMessage, response: ServerResponse, path: string) {
		const stats = stat(path);
		if (!stats?.isFile()) {
			response.writeHead(404).end();
			return;
		}
		const range = request.headers.range ?? null;
		state.downloads.push({ path, range });
		while (range === null && state.hold === path) {
			state.held++;
			await new Promise<void>((resolve) => release.push(resolve));
			state.held--;
		}
		if (response.destroyed) return;
		const match = /^bytes=(\d+)-(\d*)$/.exec(range ?? '');
		const size = stats.size;
		if (match) {
			const start = Number(match[1]);
			const end = Math.min(match[2] ? Number(match[2]) : size - 1, size - 1);
			if (start <= end) {
				response.writeHead(206, {
					'content-range': `bytes ${start}-${end}/${size}`,
					'content-length': String(end - start + 1)
				});
				createReadStream(local(path), { start, end }).pipe(response);
				return;
			}
		}
		response.writeHead(200, { 'content-length': String(size) });
		createReadStream(local(path)).pipe(response);
	}

	const server = createServer(async (request, response) => {
		const url = new URL(request.url ?? '/', base);
		if (url.pathname === '/__e2e/hold') {
			if (request.method === 'POST') {
				state.hold = ((await body(request)).path as string | null) ?? null;
				const waiting = release;
				release = [];
				for (const resolve of waiting) resolve();
				response.writeHead(204).end();
			} else {
				response.writeHead(200, { 'content-type': 'application/json' });
				response.end(JSON.stringify(state));
			}
			return;
		}
		if (url.pathname.startsWith('/files/')) {
			await file(request, response, decodeURIComponent(url.pathname.slice('/files'.length)));
			return;
		}
		const data = await body(request);
		const path = typeof data.path === 'string' ? data.path : '';
		switch (url.pathname) {
			case '/api/auth/login':
				if (data.username !== OPENLIST_USER || data.password !== OPENLIST_PASSWORD) {
					reply(response, null, 400, 'password is incorrect');
				} else {
					reply(response, { token: 'token-1' });
				}
				return;
			case '/api/fs/list': {
				if (!stat(path)?.isDirectory()) {
					reply(response, null, 500, 'failed get objs: object not found');
					return;
				}
				const content = readdirSync(local(path))
					.sort()
					.map((name) => entry(name, statSync(join(local(path), name))));
				reply(response, { content, total: content.length, readme: '', header: '', write: false, provider: 'Fake' });
				return;
			}
			case '/api/fs/get': {
				const stats = stat(path);
				if (!stats) {
					reply(response, null, 500, 'failed get obj: object not found');
					return;
				}
				const name = path.split('/').filter(Boolean).at(-1) ?? '';
				reply(response, { ...entry(name, stats), raw_url: `${base}files${encodeURI(path)}` });
				return;
			}
			case '/api/fs/link':
				if (!stat(path)?.isFile()) {
					reply(response, null, 500, 'failed link: object not found');
					return;
				}
				reply(response, {
					url: `${base}files${encodeURI(path)}`,
					header: {},
					Expiration: null,
					concurrency: 0,
					part_size: 0,
					content_length: 0
				});
				return;
			default:
				response.writeHead(404).end();
		}
	});
	await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
	base = `http://127.0.0.1:${(server.address() as AddressInfo).port}/`;
	return {
		url: base,
		close: async () => {
			state.hold = null;
			for (const resolve of release) resolve();
			server.closeAllConnections();
			await new Promise((resolve) => server.close(resolve));
		}
	};
}

/** The fake as the tests see it, from the URL the global setup exported. */
export const openlist = {
	/** Holds whole-file downloads of `path` (as OpenList names it), or releases them with null. */
	async hold(path: string | null): Promise<void> {
		const response = await fetch(new URL('__e2e/hold', setupEnv(OPENLIST_URL_ENV)), {
			method: 'POST',
			body: JSON.stringify({ path })
		});
		if (response.status !== 204) throw new Error('fake OpenList refused the hold');
	},
	async state(): Promise<OpenListState> {
		return (await (await fetch(new URL('__e2e/hold', setupEnv(OPENLIST_URL_ENV)))).json()) as OpenListState;
	}
};
