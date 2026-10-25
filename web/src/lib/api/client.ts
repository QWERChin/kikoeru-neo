import { ApiError, CLIENT_ERROR_CODES, toApiError } from './errors';
import { session } from '../stores/session.svelte';
import {
	fetchStreamTransport,
	fetchTransport,
	type ApiRequest,
	type StreamTransport,
	type Transport
} from './transport';

// Vite replaces the env lookup with a literal at build time, so without the
// flag this branch and the mock chunk with the samples are dropped.
const transportPromise: Promise<{ request: Transport; stream: StreamTransport }> =
	import.meta.env.VITE_API_MOCK === '1'
		? import('./mock').then((m) => ({ request: m.mockTransport, stream: m.mockStreamTransport }))
		: Promise.resolve({ request: fetchTransport, stream: fetchStreamTransport });

let onUnauthorized: () => void = () => {};

/** Registers what happens after a 401 clears the session, usually a redirect to /login. */
export function setUnauthorizedHandler(handler: () => void): void {
	onUnauthorized = handler;
}

let shellToken: () => Promise<string | null> = async () => null;

/**
 * Registers where a newer token of the same account may be found. The
 * desktop shell renews the local page's session in storage, so a 401 there
 * first tries the stored token (book/src/desktop.md).
 */
export function setTokenSource(source: () => Promise<string | null>): void {
	shellToken = source;
}

/**
 * True when a newer token of the same account replaced `rejected`, so the
 * request is worth one more try. A request is never replayed for an
 * account that signed in after it was sent.
 */
async function renewed(rejected: string, account: number): Promise<boolean> {
	if (session.account !== account) return false;
	const stored = await shellToken();
	if (stored === null || stored === rejected || session.account !== account) return false;
	if (session.token === rejected) session.renewToken(stored);
	return session.token === stored;
}

export interface RequestOptions extends Omit<ApiRequest, 'token'> {
	/** Send the API token and treat a 401 as an expired session. */
	auth?: boolean;
}

// Only expire the session the request was sent with. A late 401 from a
// previous login must not sign out the account that replaced it.
function expireOn401(status: number, token: string | null): void {
	if (status === 401 && token !== null && session.token === token) {
		session.clear();
		onUnauthorized();
	}
}

export function request<T>(options: RequestOptions): Promise<T> {
	return send<T>(options, false);
}

async function send<T>({ auth = true, ...rest }: RequestOptions, retried: boolean): Promise<T> {
	const token = auth ? session.token : null;
	const account = session.account;
	const transport = (await transportPromise).request;

	let response;
	try {
		response = await transport({ ...rest, token });
	} catch {
		throw new ApiError(0, CLIENT_ERROR_CODES.network, '无法连接到服务器');
	}

	if (response.status >= 200 && response.status < 300) {
		return response.body as T;
	}

	const error = toApiError(response.status, response.body);
	// A wrong current password on a password change is also a 401, but the
	// server accepted the token, so the session stays and nothing is retried.
	const expired = auth && response.status === 401 && error.code !== 'invalid_credentials';
	if (expired && token !== null && !retried && (await renewed(token, account))) {
		return send<T>({ auth, ...rest }, true);
	}
	if (expired) expireOn401(response.status, token);
	throw error;
}

/**
 * Opens an authenticated event stream and returns its body. Errors are
 * ApiErrors like those of `request`, except that aborting rethrows the
 * abort so a closed watcher can tell it apart from a failure.
 */
export function openStream(path: string, signal: AbortSignal): Promise<ReadableStream<Uint8Array>> {
	return openStreamOnce(path, signal, false);
}

async function openStreamOnce(
	path: string,
	signal: AbortSignal,
	retried: boolean
): Promise<ReadableStream<Uint8Array>> {
	const token = session.token;
	const account = session.account;
	const transport = (await transportPromise).stream;

	let response: Response;
	try {
		response = await transport({ path, token, signal });
	} catch (e) {
		if (signal.aborted) throw e;
		throw new ApiError(0, CLIENT_ERROR_CODES.network, '无法连接到服务器');
	}

	if (!response.ok) {
		let body: unknown = null;
		try {
			body = await response.json();
		} catch {
			// toApiError reports bad_response for a body that is not JSON.
		}
		if (response.status === 401 && token !== null && !retried && (await renewed(token, account))) {
			return openStreamOnce(path, signal, true);
		}
		expireOn401(response.status, token);
		throw toApiError(response.status, body);
	}
	const type = response.headers.get('Content-Type') ?? '';
	if (!type.startsWith('text/event-stream') || !response.body) {
		// A proxy that rewrites the stream answers like this. The watcher
		// then falls back to polling the snapshot.
		await response.body?.cancel();
		throw new ApiError(response.status, CLIENT_ERROR_CODES.badResponse, '服务器没有返回事件流');
	}
	return response.body;
}
