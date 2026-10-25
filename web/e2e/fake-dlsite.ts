import { readFileSync } from 'node:fs';
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';
import { DLSITE_URL_ENV, setupEnv } from './env';

/**
 * A stand-in for the two DLsite endpoints and the cover host the metadata
 * provider reads, as in crates/neo-server/tests/process_m4.rs. Every RJ code
 * gets an answer built from the code, so each test can add its own work.
 *
 * The tests run in other processes, so they switch it through `/__e2e`:
 * `down` answers 503 as an outage would, and `hold` keeps every request
 * waiting until the mode changes, which parks a scan in its metadata stage
 * for exactly as long as a test needs.
 */
export type DlsiteMode = 'up' | 'down' | 'hold';

export interface DlsiteState {
	mode: DlsiteMode;
	/** Bumped by a test to make DLsite answer with a new title. */
	edition: number;
	/** Requests waiting in hold mode right now. */
	held: number;
	requests: { kind: 'product' | 'info' | 'cover'; code: string }[];
}

export const DLSITE_CIRCLE = 'E2E 抓取社团';
export const DLSITE_VAS = ['E2E 声优甲', 'E2E 声优乙'];
export const DLSITE_TAGS = ['ASMR', 'School / Academy'];
export const DLSITE_RELEASE = '2024-05-01';
export const DLSITE_RATING = { average: 4.62, count: 318 };
/** The size of the cover JPEG the global setup makes, so a test can tell it was served. */
export const DLSITE_COVER = { width: 120, height: 90 };

export function dlsiteTitle(code: string, edition = 1): string {
	return `E2E 抓取作品 ${code}${edition > 1 ? ` 第${edition}版` : ''}`;
}

function product(base: string, code: string, edition: number) {
	return [
		{
			workno: code,
			product_name: `${dlsiteTitle(code, edition)} [${DLSITE_CIRCLE}] | DLsite`,
			maker_id: 'RG10001',
			maker_name: DLSITE_CIRCLE,
			age_category: 3,
			regist_date: `${DLSITE_RELEASE} 16:00:00`,
			genres: [
				{ id: 1, name: 'School / Academy', search_val: '001' },
				{ id: 497, name: 'ASMR', search_val: '497' }
			],
			creaters: { voice_by: DLSITE_VAS.map((name, i) => ({ id: String(i + 1), name })) },
			image_main: { url: `${base}images/${code}_img_main.jpg` }
		}
	];
}

function info(code: string) {
	return {
		[code]: {
			dl_count: '1,234',
			rate_average_2dp: DLSITE_RATING.average,
			rate_count: DLSITE_RATING.count,
			rate_count_detail: [{ review_point: 5, count: 300, ratio: 94 }],
			rank: [],
			price: 1320
		}
	};
}

async function body(request: IncomingMessage): Promise<string> {
	let text = '';
	for await (const chunk of request) text += chunk;
	return text;
}

export interface FakeDlsite {
	url: string;
	close(): Promise<void>;
}

/** Starts the fake on a free port. `coverPath` is the JPEG every cover URL returns. */
export async function startFakeDlsite(coverPath: string): Promise<FakeDlsite> {
	const cover = readFileSync(coverPath);
	const state: DlsiteState = { mode: 'up', edition: 1, held: 0, requests: [] };
	let release: (() => void)[] = [];
	let base = '';

	const send = (response: ServerResponse, status: number, type: string, data: string | Buffer) => {
		if (response.destroyed) return;
		response.writeHead(status, { 'content-type': type });
		response.end(data);
	};
	const json = (response: ServerResponse, data: unknown) =>
		send(response, 200, 'application/json', JSON.stringify(data));

	// Waits out hold mode, then tells whether the request should fail.
	async function down(): Promise<boolean> {
		while (state.mode === 'hold') {
			state.held++;
			await new Promise<void>((resolve) => release.push(resolve));
			state.held--;
		}
		return state.mode === 'down';
	}

	const server = createServer(async (request, response) => {
		const url = new URL(request.url ?? '/', base);
		if (url.pathname === '/__e2e/state') {
			if (request.method === 'POST') {
				const change = JSON.parse(await body(request)) as Partial<Pick<DlsiteState, 'mode' | 'edition'>>;
				Object.assign(state, change);
				if (state.mode !== 'hold') {
					const waiting = release;
					release = [];
					for (const resolve of waiting) resolve();
				}
				response.writeHead(204).end();
				return;
			}
			json(response, state);
			return;
		}
		let kind: DlsiteState['requests'][number]['kind'];
		let code: string;
		if (url.pathname === '/maniax/api/=/product.json') {
			kind = 'product';
			code = url.searchParams.get('workno') ?? '';
		} else if (url.pathname === '/maniax/product/info/ajax') {
			kind = 'info';
			code = url.searchParams.get('product_id') ?? '';
		} else if (url.pathname.startsWith('/images/')) {
			kind = 'cover';
			code = url.pathname.slice('/images/'.length).split('_')[0] ?? '';
		} else {
			send(response, 404, 'text/plain', 'not found');
			return;
		}
		state.requests.push({ kind, code });
		if (await down()) {
			send(response, 503, 'text/plain', 'service unavailable');
			return;
		}
		if (kind === 'product') json(response, product(base, code, state.edition));
		else if (kind === 'info') json(response, info(code));
		else send(response, 200, 'image/jpeg', cover);
	});
	await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
	base = `http://127.0.0.1:${(server.address() as AddressInfo).port}/`;
	return {
		url: base,
		close: async () => {
			state.mode = 'down';
			for (const resolve of release) resolve();
			server.closeAllConnections();
			await new Promise((resolve) => server.close(resolve));
		}
	};
}

/** The fake as the tests see it, from the URL the global setup exported. */
export const dlsite = {
	async set(change: Partial<Pick<DlsiteState, 'mode' | 'edition'>>): Promise<void> {
		const response = await fetch(new URL('__e2e/state', setupEnv(DLSITE_URL_ENV)), {
			method: 'POST',
			body: JSON.stringify(change)
		});
		if (response.status !== 204) throw new Error(`fake DLsite refused ${JSON.stringify(change)}`);
	},
	async state(): Promise<DlsiteState> {
		return (await (await fetch(new URL('__e2e/state', setupEnv(DLSITE_URL_ENV)))).json()) as DlsiteState;
	}
};
