// Mock state for the roots API (book/src/contract.md, "媒体库管理接口"):
// the contract/m7 and contract/webdav samples, plus create, update and delete with the errors
// the server gives. Loaded only in mock mode, through mock.ts.
//
// Values that trigger errors, so pages and the mock Playwright suite can
// reach every branch:
// - an OpenList or WebDAV host containing `unreachable` answers 503 storage_unavailable,
// - the OpenList or WebDAV password `wrong password` answers 502 upstream_error,
// - the local path MOCK_NOT_A_DIR answers 400 invalid_request,
// and every check passes with `skip_check`.
import roots from '../../../../contract/m7/roots.json';
import webdavRoots from '../../../../contract/webdav/roots.json';
import type { ApiRequest, ApiResponse } from './transport';
import type { ErrorCode, ErrorResponse, RootInputSettings, RootSettings, RootStatus } from './types';
import { markRootMissing } from './mock-admin';
import { catalog } from './mock-catalog';
import { scanRunningFor } from './mock-tasks';

export const MOCK_UNREACHABLE_HOST = 'unreachable';
export const MOCK_WRONG_PASSWORD = 'wrong password';
export const MOCK_NOT_A_DIR = '/srv/media/不存在';

/** Library status, changed by scans and by the handlers below. */
export const rootsState: RootStatus[] = structuredClone([...roots, ...webdavRoots] as RootStatus[]);

/**
 * Bodies of the create and update requests, with any password replaced by
 * `<redacted>`, so the mock Playwright suite can check what was sent
 * without the page's secret being kept anywhere.
 */
const rootRequests: { method: string; path: string; body: unknown }[] = [];
(globalThis as { __kikoeruMockRootRequests?: unknown[] }).__kikoeruMockRootRequests = rootRequests;

function redacted(body: unknown): unknown {
	const copy = structuredClone(body) as { settings?: { password?: unknown } } | null;
	const settings = copy?.settings;
	if (settings && typeof settings.password === 'string') settings.password = '<redacted>';
	return copy;
}

function errorResponse(status: number, code: ErrorCode, message: string): ApiResponse {
	return { status, body: { error: { code, message } } satisfies ErrorResponse };
}

const ROOT_ID = /^[a-z0-9][a-z0-9_-]{0,31}$/;
const AUTO_SCAN_MIN = { local: 300, openlist: 1800, webdav: 1800 } as const;
const AUTO_SCAN_MAX = 30 * 86_400;

function origin(url: string): string | null {
	try {
		const u = new URL(url);
		if (u.protocol !== 'http:' && u.protocol !== 'https:') return null;
		return `${u.protocol}//${u.hostname}:${u.port || (u.protocol === 'https:' ? '443' : '80')}`;
	} catch {
		return null;
	}
}

interface Input {
	display_name?: unknown;
	settings?: RootInputSettings;
	skip_check?: boolean | null;
}

/** The 400 of a malformed body, or null. */
function invalidInput({ display_name, settings }: Input): string | null {
	if (typeof display_name !== 'string' || display_name.trim() === '') return '名称不能为空';
	if ([...display_name].length > 100) return '名称最多 100 个字';
	const interval = settings?.auto_scan_interval ?? null;
	if (settings && interval !== null && (interval < AUTO_SCAN_MIN[settings.type] || interval > AUTO_SCAN_MAX)) {
		return `auto_scan_interval 要在 ${AUTO_SCAN_MIN[settings.type]} 到 ${AUTO_SCAN_MAX} 秒之间`;
	}
	if (settings && settings.type !== 'local' && (settings as { watch?: unknown }).watch != null) {
		return `${LABEL[settings.type]} 媒体库不能监听目录`;
	}
	if (settings?.type === 'local') {
		if (!settings.path.startsWith('/')) return '路径必须是绝对路径';
		if (settings.path === MOCK_NOT_A_DIR) return `路径 ${settings.path} 不存在或不是目录`;
		return null;
	}
	if (settings?.type === 'openlist' || settings?.type === 'webdav') {
		if (origin(settings.url) === null) return `${LABEL[settings.type]} 的地址必须是 http 或 https`;
		if (typeof settings.password === 'string' && /[\r\n]/.test(settings.password)) return '密码不能包含换行';
		return null;
	}
	return '不认识的媒体库类型';
}

const LABEL = { openlist: 'OpenList', webdav: 'WebDAV' } as const;

/** The 502 or 503 of a failed OpenList login or WebDAV PROPFIND, or null. */
function failedCheck(settings: RootInputSettings, password: string | null): ApiResponse | null {
	if (settings.type === 'local') return null;
	const label = LABEL[settings.type];
	if (new URL(settings.url).hostname.includes(MOCK_UNREACHABLE_HOST)) {
		return errorResponse(503, 'storage_unavailable', `连不上 ${label} ${settings.url}：连接超时`);
	}
	if (password === MOCK_WRONG_PASSWORD) {
		return errorResponse(502, 'upstream_error', `${label} 登录失败：用户名或密码不对`);
	}
	return null;
}

// The mock plays the server, whose default for `watch` is off.
function shownSettings(settings: RootInputSettings, passwordSet: boolean): RootSettings {
	const auto_scan_interval = settings.auto_scan_interval ?? null;
	if (settings.type === 'local') return { type: 'local', path: settings.path, watch: settings.watch ?? false, auto_scan_interval };
	if (settings.type === 'webdav') {
		return {
			type: 'webdav',
			url: settings.url,
			base_path: settings.base_path,
			username: settings.username,
			password_set: passwordSet,
			list_concurrency: settings.list_concurrency ?? null,
			auto_scan_interval
		};
	}
	return {
		type: 'openlist',
		url: settings.url,
		base_path: settings.base_path,
		username: settings.username,
		password_set: passwordSet,
		list_concurrency: settings.list_concurrency ?? null,
		link_rate: settings.link_rate ?? null,
		auto_scan_interval
	};
}

/** What a scheduler and watcher would report for these settings. */
function schedule(settings: RootSettings): Pick<RootStatus, 'next_scan_at' | 'watching'> {
	const interval = settings.auto_scan_interval;
	return {
		next_scan_at: interval === null ? null : Date.now() + interval * 1000,
		watching: settings.type === 'local' && settings.watch
	};
}

function handleCreate(body: unknown): ApiResponse {
	const input = (body ?? {}) as Input & { id?: unknown };
	const id = input.id;
	if (typeof id !== 'string' || !ROOT_ID.test(id)) return errorResponse(400, 'invalid_request', 'ID 不合法');
	const invalid = invalidInput(input);
	if (invalid) return errorResponse(400, 'invalid_request', invalid);
	const settings = input.settings!;
	const password = settings.type !== 'local' ? (settings.password ?? null) : null;
	if (settings.type !== 'local' && !password) return errorResponse(400, 'invalid_request', `${LABEL[settings.type]} 媒体库要填密码`);
	if (rootsState.some((r) => r.id === id)) return errorResponse(409, 'root_exists', `已经有 ID 为 ${id} 的媒体库`);
	if (!input.skip_check) {
		const failed = failedCheck(settings, password);
		if (failed) return failed;
	}
	const shown = shownSettings(settings, password !== null);
	const root: RootStatus = {
		id,
		display_name: (input.display_name as string).trim(),
		kind: settings.type,
		status: 'unknown',
		last_scan_at: null,
		work_count: 0,
		source: 'db',
		settings: shown,
		...schedule(shown)
	};
	rootsState.push(root);
	return { status: 201, body: root };
}

/** The fields whose change during a scan answers 409 scan_running. */
function location(settings: RootSettings | RootInputSettings): string {
	return settings.type === 'local'
		? `local ${settings.path}`
		: `${settings.type} ${settings.url} ${settings.base_path} ${settings.username}`;
}

function handleUpdate(id: string, body: unknown): ApiResponse {
	const root = rootsState.find((r) => r.id === id);
	if (!root) return errorResponse(404, 'not_found', '没有这个媒体库');
	if (root.source === 'config') return errorResponse(409, 'root_read_only', '配置文件里的媒体库不能通过接口修改');
	const input = (body ?? {}) as Input;
	const invalid = invalidInput(input);
	if (invalid) return errorResponse(400, 'invalid_request', invalid);
	const settings = input.settings!;
	const before = root.settings;
	let password: string | null = null;
	let passwordSet = false;
	if (settings.type !== 'local') {
		password = settings.password ?? null;
		// Like the server, a saved password is kept only for the same kind of service.
		const kept = before.type === settings.type && before.password_set;
		if (password === null && !kept) return errorResponse(400, 'invalid_request', `${LABEL[settings.type]} 媒体库要填密码`);
		if (password === null && before.type !== 'local' && origin(before.url) !== origin(settings.url)) {
			return errorResponse(400, 'invalid_request', `${LABEL[settings.type]} 的地址换了协议、主机或端口，要重新填写密码`);
		}
		passwordSet = true;
	}
	if (location(before) !== location(settings) && scanRunningFor(id)) {
		return errorResponse(409, 'scan_running', '这个媒体库正在扫描');
	}
	if (!input.skip_check) {
		const failed = failedCheck(settings, password);
		if (failed) return failed;
	}
	root.display_name = (input.display_name as string).trim();
	root.kind = settings.type;
	root.settings = shownSettings(settings, passwordSet);
	Object.assign(root, schedule(root.settings));
	return { status: 200, body: root };
}

function handleDelete(id: string): ApiResponse {
	const at = rootsState.findIndex((r) => r.id === id);
	if (at < 0) return errorResponse(404, 'not_found', '没有这个媒体库');
	if (rootsState[at]!.source === 'config') {
		return errorResponse(409, 'root_read_only', '配置文件里的媒体库不能通过接口删除');
	}
	if (scanRunningFor(id)) return errorResponse(409, 'scan_running', '这个媒体库正在扫描或排队扫描，先取消扫描');
	rootsState.splice(at, 1);
	const works = catalog.filter((w) => w.root === id && !w.summary.missing);
	for (const work of works) work.summary.missing = true;
	markRootMissing(works.map((w) => w.summary));
	return { status: 204, body: null };
}

export function routeRoots({ method, path, body }: ApiRequest): ApiResponse | null {
	if (method === 'GET' && path === '/admin/roots') return { status: 200, body: rootsState };
	if (method === 'POST' && path === '/admin/roots') {
		rootRequests.push({ method, path, body: redacted(body) });
		return handleCreate(body);
	}
	const m = /^\/admin\/roots\/([^/]+)$/.exec(path);
	if (!m) return null;
	const id = decodeURIComponent(m[1]!);
	if (method === 'PUT') {
		rootRequests.push({ method, path, body: redacted(body) });
		return handleUpdate(id, body);
	}
	if (method === 'DELETE') return handleDelete(id);
	return null;
}
