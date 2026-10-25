// Mock state for the server settings API (book/src/contract.md, "服务器设置").
// Loaded only in mock mode, through mock.ts.
//
// A proxy whose host is MOCK_UNREACHABLE_PROXY_HOST fails the connection
// check with 502 upstream_error, so pages and the mock Playwright suite can
// reach the failure branch.
import type { ApiRequest, ApiResponse } from './transport';
import type { ErrorCode, ErrorResponse, ProxyCheckResult, ServerSettings } from './types';

export const MOCK_UNREACHABLE_PROXY_HOST = 'unreachable.test';

const MOCK_CHECK: ProxyCheckResult = { url: 'https://www.dlsite.com/', status: 200, elapsed_ms: 380 };

const SCHEMES = ['http', 'https', 'socks5', 'socks5h'];
// Only these two are dropped from the saved form, as the contract says.
const DEFAULT_PORTS: Record<string, string> = { http: '80', https: '443' };

export type NormalisedProxy = { ok: true; value: string | null; host: string | null } | { ok: false; message: string };

const invalid = (message: string): NormalisedProxy => ({ ok: false, message });

/** Applies the contract's saving rules to `metadata_proxy`, roughly like the server. */
export function normaliseProxy(raw: unknown): NormalisedProxy {
	if (raw === null || raw === undefined) return { ok: true, value: null, host: null };
	if (typeof raw !== 'string') return invalid('metadata_proxy 必须是字符串或 null');
	const text = raw.trim();
	if (text === '') return { ok: true, value: null, host: null };
	const parts = /^([^:/?#]+):\/\/([^/?#]*)(.*)$/.exec(text);
	if (!parts) return invalid('代理地址要写成“协议://主机:端口”，比如 http://127.0.0.1:7890');
	const scheme = parts[1]!.toLowerCase();
	const authority = parts[2]!;
	if (!SCHEMES.includes(scheme)) return invalid(`不支持 ${scheme} 代理，只能用 http、https、socks5 或 socks5h`);
	if (authority.includes('@')) return invalid('代理地址不能带用户名和密码');
	if (parts[3] !== '' && parts[3] !== '/') {
		return invalid('代理地址只能有协议、主机和端口，不能有路径、查询参数或 # 后面的部分');
	}
	const hostPort = /^(\[[0-9a-fA-F:.]+\]|[^:[\]]+)(?::(\d{1,5}))?$/.exec(authority);
	const port = hostPort?.[2];
	if (!hostPort || (port !== undefined && (Number(port) < 1 || Number(port) > 65535))) {
		return invalid('代理地址的主机名或端口不对');
	}
	const host = hostPort[1]!.toLowerCase();
	const shownPort = port === undefined || DEFAULT_PORTS[scheme] === String(Number(port)) ? '' : `:${Number(port)}`;
	return { ok: true, value: `${scheme}://${host}${shownPort}`, host };
}

let stored: string | null = null;

function errorResponse(status: number, code: ErrorCode, message: string): ApiResponse {
	return { status, body: { error: { code, message } } satisfies ErrorResponse };
}

function proxyOf(body: unknown): NormalisedProxy {
	return normaliseProxy((body as Partial<ServerSettings> | null)?.metadata_proxy);
}

export function routeSettings({ method, path, body }: ApiRequest): ApiResponse | null {
	if (path === '/admin/settings' && method === 'GET') {
		return { status: 200, body: { metadata_proxy: stored } satisfies ServerSettings };
	}
	if (path === '/admin/settings' && method === 'PUT') {
		const proxy = proxyOf(body);
		if (!proxy.ok) return errorResponse(400, 'invalid_request', proxy.message);
		stored = proxy.value;
		return { status: 200, body: { metadata_proxy: stored } satisfies ServerSettings };
	}
	if (path === '/admin/settings/proxy-check' && method === 'POST') {
		const proxy = proxyOf(body);
		if (!proxy.ok) return errorResponse(400, 'invalid_request', proxy.message);
		if (proxy.host === MOCK_UNREACHABLE_PROXY_HOST) {
			return errorResponse(502, 'upstream_error', '连不上 DLsite：tcp connect error: Connection refused (os error 111)');
		}
		return { status: 200, body: MOCK_CHECK };
	}
	return null;
}
