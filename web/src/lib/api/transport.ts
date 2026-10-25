export type HttpMethod = 'GET' | 'POST' | 'PUT' | 'DELETE';

export interface ApiRequest {
	method: HttpMethod;
	/** Path below `/api`, starting with a slash. */
	path: string;
	query?: Record<string, string | number | boolean | undefined>;
	body?: unknown;
	token?: string | null;
	/**
	 * `text` returns a successful body as a string, for WebVTT subtitles.
	 * `bytes` returns a successful body that is not JSON
	 * as a Uint8Array, for the waveform, whose 202 answer is still JSON.
	 */
	responseType?: 'json' | 'text' | 'bytes';
	/** Lets the request outlive the page, for the last report before unload. */
	keepalive?: boolean;
}

export interface ApiResponse {
	status: number;
	/** Parsed JSON, or null for 204 and for bodies that are not JSON. */
	body: unknown;
}

/**
 * Sends one request and returns the raw outcome. The real and the mock
 * transport share this shape so error handling lives in one place.
 */
export type Transport = (request: ApiRequest) => Promise<ApiResponse>;

export function buildUrl(path: string, query?: ApiRequest['query']): string {
	const params = new URLSearchParams();
	for (const [key, value] of Object.entries(query ?? {})) {
		if (value !== undefined) params.set(key, String(value));
	}
	const search = params.toString();
	return `/api${path}${search ? `?${search}` : ''}`;
}

const ACCEPT: Record<NonNullable<ApiRequest['responseType']>, string> = {
	json: 'application/json',
	text: 'text/vtt, text/plain, application/json',
	bytes: 'application/octet-stream, application/json'
};

function isJson(response: Response): boolean {
	return (response.headers.get('Content-Type') ?? '').startsWith('application/json');
}

export const fetchTransport: Transport = async ({
	method,
	path,
	query,
	body,
	token,
	responseType = 'json',
	keepalive
}) => {
	const headers: Record<string, string> = { Accept: ACCEPT[responseType] };
	if (token) headers.Authorization = `Bearer ${token}`;
	if (body !== undefined) headers['Content-Type'] = 'application/json';

	const response = await fetch(buildUrl(path, query), {
		method,
		headers,
		body: body === undefined ? undefined : JSON.stringify(body),
		keepalive
	});

	if (response.status === 204) return { status: 204, body: null };
	if (responseType === 'bytes' && response.ok && !isJson(response)) {
		return { status: response.status, body: new Uint8Array(await response.arrayBuffer()) };
	}
	const text = await response.text();
	// Errors are JSON even for text endpoints, so only a success stays text.
	if (responseType === 'text' && response.ok) return { status: response.status, body: text };
	let parsed: unknown = null;
	try {
		parsed = text ? JSON.parse(text) : null;
	} catch {
		// Left as null so the caller reports bad_response with the HTTP status.
	}
	return { status: response.status, body: parsed };
};

export interface StreamRequest {
	/** Path below `/api`, starting with a slash. */
	path: string;
	token: string | null;
	signal: AbortSignal;
}

/**
 * Opens a `text/event-stream` response. The caller reads the body itself,
 * so this returns the raw Response instead of a parsed body.
 */
export type StreamTransport = (request: StreamRequest) => Promise<Response>;

export const fetchStreamTransport: StreamTransport = ({ path, token, signal }) => {
	const headers: Record<string, string> = { Accept: 'text/event-stream' };
	if (token) headers.Authorization = `Bearer ${token}`;
	return fetch(buildUrl(path), { headers, signal, cache: 'no-store' });
};
