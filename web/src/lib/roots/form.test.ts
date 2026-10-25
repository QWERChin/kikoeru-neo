import { describe, expect, it } from 'vitest';
import sampleRoots from '../../../../contract/m7/roots.json';
import webdavRoots from '../../../../contract/webdav/roots.json';
import type { RootStatus } from '../api/types';
import {
	AUTO_SCAN_MAX,
	autoScanChoices,
	autoScanError,
	emptyForm,
	formatInterval,
	formFromRoot,
	isAbsolutePath,
	passwordNeeded,
	rootIdError,
	suggestRootId,
	toCreateRequest,
	toUpdateRequest,
	urlOrigin,
	type RootForm
} from './form';

const [configRoot, dbRoot] = sampleRoots as [RootStatus, RootStatus];

function openListForm(overrides: Partial<RootForm> = {}): RootForm {
	return {
		...emptyForm('openlist'),
		displayName: '网盘',
		id: 'cloud',
		url: 'https://openlist.example.lan',
		basePath: '/cloud/云端作品',
		username: 'kikoeru',
		password: 'secret',
		...overrides
	};
}

describe('suggestRootId', () => {
	it('lowercases ASCII and joins the rest with single dashes', () => {
		expect(suggestRootId('My Voice Library')).toBe('my-voice-library');
		expect(suggestRootId('  Local / 本地作品  ')).toBe('local');
		expect(suggestRootId('a__b--c')).toBe('a__b-c');
	});

	it('folds full-width letters and drops accents', () => {
		expect(suggestRootId('ＡＳＭＲ')).toBe('asmr');
		expect(suggestRootId('Café')).toBe('cafe');
		expect(suggestRootId('Crème brûlée')).toBe('creme-brulee');
	});

	it('falls back to library for a name without ASCII', () => {
		expect(suggestRootId('音声')).toBe('library');
		expect(suggestRootId('')).toBe('library');
	});

	it('picks a free ID next to config and database roots', () => {
		expect(suggestRootId('音声', ['library'])).toBe('library-2');
		expect(suggestRootId('Local', ['local', 'local-2'])).toBe('local-3');
	});

	it('stays within 32 characters, suffix included, and never ends in a dash', () => {
		const long = 'a'.repeat(31) + ' b';
		expect(suggestRootId(long)).toBe('a'.repeat(31));
		const id = suggestRootId('x'.repeat(40), ['x'.repeat(32)]);
		expect(id).toBe(`${'x'.repeat(30)}-2`);
		expect(id).toHaveLength(32);
		expect(rootIdError(id)).toBeNull();
	});

	it('always suggests an ID the server accepts', () => {
		for (const name of ['-_-', 'Ä Ö Ü', '1.2.3', 'ＬＯＣＡＬ１', '音声 Vol.2']) {
			expect(rootIdError(suggestRootId(name))).toBeNull();
		}
	});
});

describe('rootIdError', () => {
	it('follows the contract rule', () => {
		expect(rootIdError('local')).toBeNull();
		expect(rootIdError('a-b_c')).toBeNull();
		expect(rootIdError('')).not.toBeNull();
		expect(rootIdError('Local')).not.toBeNull();
		expect(rootIdError('-voice')).not.toBeNull();
		expect(rootIdError('音声')).not.toBeNull();
		expect(rootIdError('a'.repeat(33))).not.toBeNull();
	});
});

describe('paths and addresses', () => {
	it('accepts Unix, Windows drive and UNC paths only', () => {
		expect(isAbsolutePath('/srv/media')).toBe(true);
		expect(isAbsolutePath('C:\\Music')).toBe(true);
		expect(isAbsolutePath('d:/voice')).toBe(true);
		expect(isAbsolutePath('\\\\nas\\share')).toBe(true);
		expect(isAbsolutePath('media/voice')).toBe(false);
		expect(isAbsolutePath('~/voice')).toBe(false);
	});

	it('compares scheme, host and port with default ports filled in', () => {
		expect(urlOrigin('https://OpenList.example.lan/')).toBe('https://openlist.example.lan:443');
		expect(urlOrigin('https://openlist.example.lan:443/dav')).toBe('https://openlist.example.lan:443');
		expect(urlOrigin('http://192.168.1.2:5244')).toBe('http://192.168.1.2:5244');
		expect(urlOrigin('ftp://host')).toBeNull();
		expect(urlOrigin('openlist.example.lan')).toBeNull();
	});
});

describe('passwordNeeded', () => {
	it('keeps the stored password while the origin stays the same', () => {
		const form = formFromRoot(dbRoot);
		expect(passwordNeeded(form, dbRoot)).toBeNull();
		expect(passwordNeeded({ ...form, url: 'https://openlist.example.lan:443/sub' }, dbRoot)).toBeNull();
	});

	it('asks for a new password when the scheme, host or port changes', () => {
		const form = formFromRoot(dbRoot);
		for (const url of ['http://openlist.example.lan', 'https://other.example.lan', 'https://openlist.example.lan:8443']) {
			expect(passwordNeeded({ ...form, url }, dbRoot)).toMatch(/重新填写密码/);
		}
	});

	it('needs a password on create, after switching from local, and when none is stored', () => {
		expect(passwordNeeded(openListForm(), null)).not.toBeNull();
		expect(passwordNeeded(openListForm(), configRoot)).not.toBeNull();
		const unset = structuredClone(dbRoot);
		if (unset.settings.type === 'openlist') unset.settings.password_set = false;
		expect(passwordNeeded(formFromRoot(unset), unset)).not.toBeNull();
	});

	it('never applies to a local root', () => {
		expect(passwordNeeded(emptyForm('local'), null)).toBeNull();
	});
});

describe('toCreateRequest', () => {
	it('shapes a local root without a skip_check field', () => {
		const result = toCreateRequest({ ...emptyForm('local'), displayName: ' 本地 ', id: 'local', path: '/srv/media/voice' });
		expect(result).toEqual({
			ok: true,
			request: {
				id: 'local',
				display_name: '本地',
				settings: { type: 'local', path: '/srv/media/voice', watch: false, auto_scan_interval: null }
			}
		});
	});

	it('sends paths and the username exactly as typed', () => {
		const local = toCreateRequest({ ...emptyForm('local'), displayName: 'a', id: 'a', path: '/srv/media/voice ' });
		expect(local.ok && local.request.settings).toMatchObject({ type: 'local', path: '/srv/media/voice ' });
		const remote = toCreateRequest(openListForm({ basePath: '/cloud/云端作品 ', username: ' kikoeru' }));
		expect(remote.ok && remote.request.settings).toMatchObject({ base_path: '/cloud/云端作品 ', username: ' kikoeru' });
		const stored = { ...dbRoot, kind: 'local' as const, settings: { type: 'local' as const, path: '/srv/v ', watch: true, auto_scan_interval: null } };
		const renamed = toUpdateRequest({ ...formFromRoot(stored), displayName: '新名字' }, stored);
		expect(renamed.ok && renamed.request.settings).toEqual({ type: 'local', path: '/srv/v ', watch: true, auto_scan_interval: null });
	});

	it('shapes an OpenList root with defaults as null and skip_check only when asked', () => {
		const result = toCreateRequest(openListForm({ listConcurrency: '8', linkRate: '0.5' }), true);
		expect(result).toEqual({
			ok: true,
			request: {
				id: 'cloud',
				display_name: '网盘',
				settings: {
					type: 'openlist',
					url: 'https://openlist.example.lan',
					base_path: '/cloud/云端作品',
					username: 'kikoeru',
					password: 'secret',
					list_concurrency: 8,
					link_rate: 0.5,
					auto_scan_interval: null
				},
				skip_check: true
			}
		});
		const plain = toCreateRequest(openListForm());
		expect(plain.ok && plain.request.settings).toMatchObject({ list_concurrency: null, link_rate: null });
		expect(plain.ok && 'skip_check' in plain.request).toBe(false);
	});

	it('keeps spaces in the password', () => {
		const result = toCreateRequest(openListForm({ password: ' pass word ' }));
		expect(result.ok && result.request.settings.type === 'openlist' && result.request.settings.password).toBe(' pass word ');
	});

	it('reports each bad field', () => {
		const result = toCreateRequest({
			...openListForm(),
			displayName: ' ',
			id: 'Bad',
			url: 'openlist.lan',
			basePath: '115',
			username: '',
			password: '',
			listConcurrency: '0',
			linkRate: '-1'
		});
		expect(result.ok).toBe(false);
		expect(!result.ok && Object.keys(result.errors).sort()).toEqual(
			['basePath', 'displayName', 'id', 'linkRate', 'listConcurrency', 'password', 'url', 'username'].sort()
		);
	});

	it('rejects a password with a line break', () => {
		for (const password of ['a\nb', 'a\r\nb', 'a\rb']) {
			const result = toCreateRequest(openListForm({ password }));
			expect(!result.ok && result.errors.password).toBe('密码不能包含换行');
		}
	});

	it('rejects relative paths and too long names', () => {
		const result = toCreateRequest({ ...emptyForm('local'), displayName: '名'.repeat(101), id: 'x', path: 'voice' });
		expect(!result.ok && Object.keys(result.errors).sort()).toEqual(['displayName', 'path']);
		const ok = toCreateRequest({ ...emptyForm('local'), displayName: '名'.repeat(100), id: 'x', path: '/v' });
		expect(ok.ok).toBe(true);
	});
});

describe('toUpdateRequest', () => {
	it('sends a null password to keep the stored one, and no id', () => {
		const result = toUpdateRequest({ ...formFromRoot(dbRoot), displayName: '云端作品 2' }, dbRoot);
		expect(result).toEqual({
			ok: true,
			request: {
				display_name: '云端作品 2',
				settings: {
					type: 'openlist',
					url: 'https://openlist.example.lan',
					base_path: '/cloud/云端作品',
					username: 'kikoeru',
					password: null,
					list_concurrency: null,
					link_rate: null,
					auto_scan_interval: 21600
				}
			}
		});
	});

	it('refuses a host change without a password, and sends the new one with it', () => {
		const moved = { ...formFromRoot(dbRoot), url: 'https://new.example.lan' };
		const refused = toUpdateRequest(moved, dbRoot);
		expect(!refused.ok && refused.errors.password).toMatch(/重新填写密码/);
		const sent = toUpdateRequest({ ...moved, password: 'new secret' }, dbRoot, true);
		expect(sent.ok && sent.request).toMatchObject({
			settings: { url: 'https://new.example.lan', password: 'new secret' },
			skip_check: true
		});
	});

	it('round-trips the advanced settings of a stored root', () => {
		const tuned = structuredClone(dbRoot);
		if (tuned.settings.type === 'openlist') Object.assign(tuned.settings, { list_concurrency: 2, link_rate: 0.25 });
		const result = toUpdateRequest(formFromRoot(tuned), tuned);
		expect(result.ok && result.request.settings).toMatchObject({ list_concurrency: 2, link_rate: 0.25 });
	});

	it('turns a local root into OpenList only with a password', () => {
		const local = { ...dbRoot, kind: 'local' as const, settings: { type: 'local' as const, path: '/srv/v', watch: false, auto_scan_interval: null } };
		const form = { ...formFromRoot(local), kind: 'openlist' as const, url: 'https://o.lan', username: 'u' };
		expect(toUpdateRequest(form, local).ok).toBe(false);
		expect(toUpdateRequest({ ...form, password: 'p' }, local).ok).toBe(true);
	});
});

describe('formFromRoot', () => {
	it('starts with an empty password and the stored values', () => {
		const form = formFromRoot(dbRoot);
		expect(form).toMatchObject({ kind: 'openlist', id: 'cloud', idEdited: true, password: '', basePath: '/cloud/云端作品' });
		expect(formFromRoot(configRoot)).toMatchObject({ kind: 'local', path: '/srv/media/works' });
	});
});

describe('automatic scans and watching', () => {
	it('formats intervals as the sheet shows them', () => {
		expect(formatInterval(300)).toBe('5 分钟');
		expect(formatInterval(21_600)).toBe('6 小时');
		expect(formatInterval(86_400)).toBe('1 天');
		expect(formatInterval(5400)).toBe('1 小时 30 分钟');
	});

	it('offers only intervals at or above the kind minimum, and keeps a stored odd one', () => {
		expect(autoScanChoices('local', null)[0]).toBe(300);
		expect(autoScanChoices('openlist', null)[0]).toBe(1800);
		expect(autoScanChoices('openlist', 7200)).toContain(7200);
		expect(autoScanChoices('local', 300).filter((s) => s === 300)).toHaveLength(1);
	});

	it('enforces the contract minimum per kind and the 30 day maximum', () => {
		expect(autoScanError('local', 300)).toBeNull();
		expect(autoScanError('local', 299)).toMatch(/最短每 5 分钟/);
		expect(autoScanError('openlist', 900)).toMatch(/OpenList.*30 分钟/);
		expect(autoScanError('openlist', AUTO_SCAN_MAX)).toBeNull();
		expect(autoScanError('openlist', AUTO_SCAN_MAX + 1)).toMatch(/30 天/);
		expect(autoScanError('openlist', null)).toBeNull();
	});

	it('refuses a local interval after switching to OpenList', () => {
		const result = toCreateRequest(openListForm({ autoScanInterval: 300 }));
		expect(!result.ok && result.errors.autoScanInterval).toMatch(/30 分钟/);
	});

	it('always sends watch and the interval, since PUT replaces every setting', () => {
		const form = { ...emptyForm('local'), displayName: 'a', id: 'a', path: '/a', watch: true, autoScanInterval: 3600 };
		const result = toCreateRequest(form);
		expect(result.ok && result.request.settings).toEqual({ type: 'local', path: '/a', watch: true, auto_scan_interval: 3600 });
		const remote = toUpdateRequest({ ...formFromRoot(dbRoot), autoScanInterval: null }, dbRoot);
		expect(remote.ok && remote.request.settings).toMatchObject({ auto_scan_interval: null });
		expect(remote.ok && 'watch' in remote.request.settings).toBe(false);
	});

	it('reads the stored values into the form', () => {
		expect(formFromRoot(configRoot)).toMatchObject({ watch: true, autoScanInterval: 86_400 });
		expect(formFromRoot(dbRoot)).toMatchObject({ watch: false, autoScanInterval: 21_600 });
	});
});

describe('WebDAV roots', () => {
	const [nasRoot] = webdavRoots as [RootStatus];
	const webdavForm = (overrides: Partial<RootForm> = {}): RootForm => ({
		...emptyForm('webdav'),
		displayName: 'NAS',
		id: 'nas-2',
		url: 'https://nas.example.lan/dav',
		basePath: '/音声',
		username: 'kikoeru',
		password: 'secret',
		...overrides
	});

	it('sends no link_rate and no watch', () => {
		const result = toCreateRequest(webdavForm({ listConcurrency: '2', linkRate: '0.5', watch: true, autoScanInterval: 3600 }));
		expect(result.ok && result.request.settings).toEqual({
			type: 'webdav',
			url: 'https://nas.example.lan/dav',
			base_path: '/音声',
			username: 'kikoeru',
			password: 'secret',
			list_concurrency: 2,
			auto_scan_interval: 3600
		});
	});

	it('needs a password on create and keeps the stored one on the same server', () => {
		const missing = toCreateRequest(webdavForm({ password: '' }));
		expect(!missing.ok && missing.errors.password).toBe('请填写密码');
		const kept = toUpdateRequest({ ...formFromRoot(nasRoot), displayName: 'NAS 2' }, nasRoot);
		expect(kept.ok && kept.request.settings).toMatchObject({ type: 'webdav', password: null });
	});

	it('asks for a new password when the host changes', () => {
		const moved = toUpdateRequest({ ...formFromRoot(nasRoot), url: 'https://other.example.lan/dav' }, nasRoot);
		expect(!moved.ok && moved.errors.password).toMatch(/重新填写密码/);
	});

	it('asks for a new password when switching between OpenList and WebDAV, even on the same host', () => {
		const toWebdav = { ...formFromRoot(dbRoot), kind: 'webdav' as const };
		const refused = toUpdateRequest(toWebdav, dbRoot);
		expect(!refused.ok && refused.errors.password).toBe('换成 WebDAV 之后要重新填写密码');
		expect(passwordNeeded({ ...formFromRoot(nasRoot), kind: 'openlist' }, nasRoot)).toBe('换成 OpenList 之后要重新填写密码');
		expect(toUpdateRequest({ ...toWebdav, password: 'p' }, dbRoot).ok).toBe(true);
	});

	it('has the 30 minute floor of a remote root', () => {
		expect(autoScanError('webdav', 1799)).toBe('WebDAV媒体库最短每 30 分钟扫描一次');
		expect(autoScanChoices('webdav', null)[0]).toBe(1800);
		const result = toCreateRequest(webdavForm({ autoScanInterval: 300 }));
		expect(!result.ok && result.errors.autoScanInterval).toMatch(/30 分钟/);
	});

	it('reads the stored settings into the form with an empty password', () => {
		expect(formFromRoot(nasRoot)).toMatchObject({
			kind: 'webdav',
			url: 'https://nas.example.lan/dav',
			basePath: '/音声',
			username: 'kikoeru',
			password: '',
			listConcurrency: '',
			autoScanInterval: 21_600
		});
	});

	it('names WebDAV in the address errors', () => {
		const result = toCreateRequest(webdavForm({ url: '' }));
		expect(!result.ok && result.errors.url).toBe('请填写 WebDAV 的地址');
		const bad = toCreateRequest(webdavForm({ url: 'nas.example.lan' }));
		expect(!bad.ok && bad.errors.url).toMatch(/https:\/\/nas\.example\.lan\/dav/);
	});
});
