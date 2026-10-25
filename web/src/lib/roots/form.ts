// The library sheet's form state and how it becomes a RootCreateRequest or
// RootUpdateRequest. The rules mirror book/src/contract.md ("媒体库管理接口"),
// so most mistakes show next to their field instead of as a server 400.
import type { RootCreateRequest, RootInputSettings, RootKind, RootStatus, RootUpdateRequest } from '../api/types';

export interface RootForm {
	kind: RootKind;
	displayName: string;
	id: string;
	/** Set once the user types an ID, so the name stops replacing it. */
	idEdited: boolean;
	path: string;
	url: string;
	basePath: string;
	username: string;
	password: string;
	/** Text as typed. Empty means the server default. */
	listConcurrency: string;
	linkRate: string;
	/** Seconds between automatic scans, null for off. */
	autoScanInterval: number | null;
	/** Local roots only. */
	watch: boolean;
}

export type RootField =
	| 'displayName'
	| 'id'
	| 'path'
	| 'url'
	| 'basePath'
	| 'username'
	| 'password'
	| 'listConcurrency'
	| 'linkRate'
	| 'autoScanInterval';

export type RootErrors = Partial<Record<RootField, string>>;

export type FormResult<T> = { ok: true; request: T } | { ok: false; errors: RootErrors };

export const ROOT_ID_MAX = 32;
export const DISPLAY_NAME_MAX = 100;
const ROOT_ID = /^[a-z0-9][a-z0-9_-]*$/;
const FALLBACK_ID = 'library';

/** book/src/contract.md, "自动扫描和目录监听": remote listings are rate limited, so their minimum is longer. */
export const AUTO_SCAN_MIN: Record<RootKind, number> = { local: 300, openlist: 1800, webdav: 1800 };
export const KIND_LABEL: Record<RootKind, string> = { local: '本地', openlist: 'OpenList', webdav: 'WebDAV' };
export const URL_EXAMPLE: Record<'openlist' | 'webdav', string> = {
	openlist: 'https://openlist.example.lan',
	webdav: 'https://nas.example.lan/dav'
};
export const AUTO_SCAN_MAX = 30 * 86_400;
const AUTO_SCAN_PRESETS = [300, 900, 1800, 3600, 3 * 3600, 6 * 3600, 12 * 3600, 86_400, 7 * 86_400];

/** "5 分钟", "6 小时", "1 天", or a mix such as "1 小时 30 分钟" for a value from config.toml. */
export function formatInterval(seconds: number): string {
	const parts: string[] = [];
	let rest = seconds;
	for (const [unit, label] of [
		[86_400, '天'],
		[3600, '小时'],
		[60, '分钟'],
		[1, '秒']
	] as const) {
		const n = Math.floor(rest / unit);
		if (n > 0) parts.push(`${n} ${label}`);
		rest -= n * unit;
	}
	return parts.length > 0 ? parts.join(' ') : '0 秒';
}

/**
 * The intervals the sheet offers for a kind, shortest first. A stored
 * value that is not a preset, such as "2h" from config.toml, is kept as
 * a choice so opening and saving the form does not change it.
 */
export function autoScanChoices(kind: RootKind, current: number | null): number[] {
	const choices = AUTO_SCAN_PRESETS.filter((s) => s >= AUTO_SCAN_MIN[kind]);
	if (current !== null && !choices.includes(current)) choices.push(current);
	return choices.sort((a, b) => a - b);
}

export function autoScanError(kind: RootKind, interval: number | null): string | null {
	if (interval === null) return null;
	const min = AUTO_SCAN_MIN[kind];
	if (interval < min) return `${KIND_LABEL[kind]}媒体库最短每 ${formatInterval(min)}扫描一次`;
	if (interval > AUTO_SCAN_MAX) return '最长每 30 天扫描一次';
	return null;
}

export function emptyForm(kind: RootKind = 'local'): RootForm {
	return {
		kind,
		displayName: '',
		id: '',
		idEdited: false,
		path: '',
		url: '',
		basePath: '/',
		username: '',
		password: '',
		listConcurrency: '',
		linkRate: '',
		autoScanInterval: null,
		watch: false
	};
}

/** The form for editing a database root. The password always starts empty. */
export function formFromRoot(root: RootStatus): RootForm {
	const s = root.settings;
	const form = {
		...emptyForm(root.kind),
		displayName: root.display_name,
		id: root.id,
		idEdited: true,
		autoScanInterval: s.auto_scan_interval
	};
	if (s.type === 'local') return { ...form, path: s.path, watch: s.watch };
	return {
		...form,
		url: s.url,
		basePath: s.base_path,
		username: s.username,
		listConcurrency: s.list_concurrency === null ? '' : String(s.list_concurrency),
		linkRate: s.type === 'openlist' && s.link_rate !== null ? String(s.link_rate) : ''
	};
}

/**
 * An ID derived from the display name: ASCII letters and digits kept and
 * lowercased, everything else turned into single dashes. A name with no
 * ASCII at all, such as "音声", falls back to `library`. A taken ID gets
 * `-2`, `-3` and so on, because config and database roots share one
 * namespace.
 */
export function suggestRootId(displayName: string, taken: readonly string[] = []): string {
	const slug = displayName
		.normalize('NFKD')
		// NFKD splits accented letters into letter and mark, so dropping the
		// marks keeps "é" as "e" instead of a dash.
		.replace(/\p{M}/gu, '')
		.toLowerCase()
		.replace(/[^a-z0-9_-]+/g, '-')
		.replace(/-{2,}/g, '-')
		.replace(/^[-_]+|[-_]+$/g, '')
		.slice(0, ROOT_ID_MAX)
		.replace(/[-_]+$/, '');
	const base = slug === '' ? FALLBACK_ID : slug;
	const used = new Set(taken);
	if (!used.has(base)) return base;
	for (let n = 2; ; n++) {
		const suffix = `-${n}`;
		const candidate = `${base.slice(0, ROOT_ID_MAX - suffix.length).replace(/[-_]+$/, '')}${suffix}`;
		if (!used.has(candidate)) return candidate;
	}
}

export function rootIdError(id: string): string | null {
	if (id === '') return '请填写 ID';
	if (id.length > ROOT_ID_MAX) return `ID 最多 ${ROOT_ID_MAX} 个字符`;
	if (!ROOT_ID.test(id)) return 'ID 只能用小写字母、数字、- 和 _，并以字母或数字开头';
	return null;
}

/** Unix (`/srv/media`), Windows drive (`C:\Music`) or UNC (`\\nas\share`) paths. */
export function isAbsolutePath(path: string): boolean {
	return path.startsWith('/') || /^[a-zA-Z]:[\\/]/.test(path) || path.startsWith('\\\\');
}

/** Scheme, host and port, with the scheme's default port filled in. Null when `url` does not parse. */
export function urlOrigin(url: string): string | null {
	let parsed: URL;
	try {
		parsed = new URL(url.trim());
	} catch {
		return null;
	}
	if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return null;
	if (parsed.hostname === '') return null;
	const port = parsed.port || (parsed.protocol === 'https:' ? '443' : '80');
	return `${parsed.protocol}//${parsed.hostname}:${port}`;
}

/** OpenList and WebDAV roots, which log in with a stored password. */
export function isRemoteKind(kind: RootKind): kind is 'openlist' | 'webdav' {
	return kind !== 'local';
}

/**
 * Why an update must carry a new password, or null when leaving it empty
 * keeps the stored one. The server keeps a saved password only for the
 * same kind of service at the same scheme, host and port, so a password
 * saved for one server is never sent to another, and answers 400 otherwise.
 */
export function passwordNeeded(form: RootForm, original: RootStatus | null): string | null {
	if (!isRemoteKind(form.kind)) return null;
	if (original === null) return '请填写密码';
	const s = original.settings;
	if (s.type === 'local' || !s.password_set) return '请填写密码';
	if (s.type !== form.kind) return `换成 ${KIND_LABEL[form.kind]} 之后要重新填写密码`;
	const before = urlOrigin(s.url);
	const after = urlOrigin(form.url);
	if (after !== null && before !== after) return '地址的协议、主机或端口变了，请重新填写密码';
	return null;
}

function positiveInteger(text: string): number | null | undefined {
	const t = text.trim();
	if (t === '') return null;
	if (!/^\d+$/.test(t)) return undefined;
	const n = Number(t);
	return n >= 1 && n <= 0xffff_ffff ? n : undefined;
}

function positiveNumber(text: string): number | null | undefined {
	const t = text.trim();
	if (t === '') return null;
	const n = Number(t);
	return Number.isFinite(n) && n > 0 ? n : undefined;
}

function settingsFrom(
	form: RootForm,
	original: RootStatus | null,
	errors: RootErrors
): RootInputSettings {
	// Paths and the username are sent exactly as typed: trailing spaces can
	// be part of a directory name, and trimming would point at another one.
	if (form.kind === 'local') {
		const path = form.path;
		if (path.trim() === '') errors.path = '请填写文件夹的路径';
		else if (!isAbsolutePath(path)) errors.path = '请填写完整的路径，比如 /srv/media/音声';
		// PUT replaces every setting, so both are always sent, never left to the default.
		return { type: 'local', path, watch: form.watch, auto_scan_interval: form.autoScanInterval };
	}
	const kind = form.kind;
	const url = form.url.trim();
	if (url === '') errors.url = `请填写 ${KIND_LABEL[kind]} 的地址`;
	else if (urlOrigin(url) === null) errors.url = `地址要以 http:// 或 https:// 开头，比如 ${URL_EXAMPLE[kind]}`;
	const basePath = form.basePath;
	if (!basePath.startsWith('/')) errors.basePath = '目录要以 / 开头，比如 /cloud/音声';
	const username = form.username;
	if (username.trim() === '') errors.username = '请填写用户名';
	// Not trimmed: spaces can be part of a password.
	const password = form.password;
	if (/[\r\n]/.test(password)) errors.password = '密码不能包含换行';
	else if (password === '') {
		const needed = passwordNeeded(form, original);
		if (needed) errors.password = needed;
	}
	const listConcurrency = positiveInteger(form.listConcurrency);
	if (listConcurrency === undefined) errors.listConcurrency = '要填一个正整数，留空用默认值 4';
	if (kind === 'webdav') {
		// WebDAV has no direct-link step, so there is no link_rate to send.
		return {
			type: 'webdav',
			url,
			base_path: basePath,
			username,
			password: password === '' ? null : password,
			list_concurrency: listConcurrency ?? null,
			auto_scan_interval: form.autoScanInterval
		};
	}
	const linkRate = positiveNumber(form.linkRate);
	if (linkRate === undefined) errors.linkRate = '要填一个大于 0 的数，留空用默认值 1';
	return {
		type: 'openlist',
		url,
		base_path: basePath,
		username,
		// Null on an update keeps the stored password.
		password: password === '' ? null : password,
		list_concurrency: listConcurrency ?? null,
		link_rate: linkRate ?? null,
		auto_scan_interval: form.autoScanInterval
	};
}

function displayNameFrom(form: RootForm, errors: RootErrors): string {
	const name = form.displayName.trim();
	if (name === '') errors.displayName = '请填写名称';
	else if ([...name].length > DISPLAY_NAME_MAX) errors.displayName = `名称最多 ${DISPLAY_NAME_MAX} 个字`;
	return name;
}

function intervalFrom(form: RootForm, errors: RootErrors): void {
	const error = autoScanError(form.kind, form.autoScanInterval);
	if (error) errors.autoScanInterval = error;
}

const hasErrors = (errors: RootErrors) => Object.keys(errors).length > 0;

export function toCreateRequest(form: RootForm, skipCheck = false): FormResult<RootCreateRequest> {
	const errors: RootErrors = {};
	const display_name = displayNameFrom(form, errors);
	intervalFrom(form, errors);
	const id = form.id.trim();
	const idError = rootIdError(id);
	if (idError) errors.id = idError;
	const settings = settingsFrom(form, null, errors);
	if (hasErrors(errors)) return { ok: false, errors };
	return { ok: true, request: { id, display_name, settings, ...(skipCheck && { skip_check: true }) } };
}

export function toUpdateRequest(
	form: RootForm,
	original: RootStatus,
	skipCheck = false
): FormResult<RootUpdateRequest> {
	const errors: RootErrors = {};
	const display_name = displayNameFrom(form, errors);
	intervalFrom(form, errors);
	const settings = settingsFrom(form, original, errors);
	if (hasErrors(errors)) return { ok: false, errors };
	return { ok: true, request: { display_name, settings, ...(skipCheck && { skip_check: true }) } };
}
