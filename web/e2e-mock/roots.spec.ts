import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page, type TestInfo } from '@playwright/test';

// Adding, editing and deleting libraries in mock mode. The mock serves the
// contract/m7 and contract/webdav samples: 本地作品 comes from config.toml,
// and 云端作品 (OpenList) and NAS (WebDAV) come from the database with stored
// passwords. Error triggers are in
// src/lib/api/mock-roots.ts. Every test loads the app afresh.

async function openSettings(page: Page) {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill('admin');
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
	await page.getByRole('navigation', { name: '主导航' }).getByRole('link', { name: '设置', exact: true }).click();
	await expect(rows(page)).toHaveCount(3);
}

async function shot(page: Page, testInfo: TestInfo, name: string) {
	const dir = process.env.KIKOERU_MOCK_SCREENSHOTS;
	if (!dir) return;
	mkdirSync(dir, { recursive: true });
	await page.screenshot({ path: join(dir, `m7-${name}-${testInfo.project.name}.png`) });
}

const rows = (page: Page) => page.getByRole('list', { name: '媒体库状态' }).getByRole('listitem');
const row = (page: Page, name: string) => rows(page).filter({ hasText: name });
const sheet = (page: Page) => page.getByRole('dialog', { name: /媒体库/ });
const field = (page: Page, label: string) => sheet(page).getByLabel(label, { exact: true });

type RootRequest = { method: string; path: string; body: Record<string, unknown> & { settings: Record<string, unknown> } };
const rootRequests = (page: Page) =>
	page.evaluate(() => (globalThis as { __kikoeruMockRootRequests?: unknown[] }).__kikoeruMockRootRequests ?? []) as Promise<
		RootRequest[]
	>;

async function startAdding(page: Page) {
	await page.getByRole('button', { name: '添加媒体库' }).click();
	await expect(sheet(page).getByRole('heading', { name: '添加媒体库' })).toBeVisible();
}

async function openAdvanced(page: Page) {
	const summary = sheet(page).getByText('高级', { exact: true });
	if (!(await field(page, 'ID').isVisible())) await summary.click();
}

test('the sheet grows from its content, which macOS WebKit needs to draw it', async ({ page }) => {
	await openSettings(page);
	await startAdding(page);
	// A 0% flex basis inside the modal dialog drew the sheet 0px high on macOS,
	// although Chromium and the Linux WebKit draw it.
	const bases = await sheet(page).evaluate((dialog) =>
		[...dialog.querySelectorAll('form, form > div')].map((el) => getComputedStyle(el).flexBasis)
	);
	expect(bases).not.toContain('0%');
	expect((await sheet(page).boundingBox())!.height).toBeGreaterThan(200);
});

test('adds a local folder with an ID suggested from its name', async ({ page }, testInfo) => {
	await openSettings(page);
	await startAdding(page);
	await expect(sheet(page).getByRole('radio', { name: '本地文件夹' })).toHaveAttribute('aria-checked', 'true');
	await field(page, '名称').fill('My Voice 2');
	await field(page, '路径').fill('/srv/media/不存在');
	await openAdvanced(page);
	await expect(field(page, 'ID')).toHaveValue('my-voice-2');
	await expect(sheet(page)).toContainText('添加之后不能修改');
	await shot(page, testInfo, 'add-local');

	// The server checks that the folder exists.
	await sheet(page).getByRole('button', { name: '添加' }).click();
	await expect(sheet(page).getByRole('alert')).toContainText('设置不正确：路径 /srv/media/不存在 不存在或不是目录');

	// An ID that a config root already uses is refused, and the message says where to change it.
	await field(page, 'ID').fill('local');
	await field(page, '路径').fill('/srv/media/voice');
	await sheet(page).getByRole('button', { name: '添加' }).click();
	await expect(sheet(page).getByRole('alert')).toContainText('已经有 ID 为“local”的媒体库');

	await field(page, 'ID').fill('my-voice');
	await sheet(page).getByRole('button', { name: '添加' }).click();
	await expect(sheet(page)).toBeHidden();
	await expect(rows(page)).toHaveCount(4);
	await expect(row(page, 'My Voice 2')).toContainText('还没扫描过');
	await expect(row(page, 'My Voice 2')).not.toContainText('由配置文件管理');
	const requests = await rootRequests(page);
	expect(requests.at(-1)).toEqual({
		method: 'POST',
		path: '/admin/roots',
		body: {
			id: 'my-voice',
			display_name: 'My Voice 2',
			settings: { type: 'local', path: '/srv/media/voice', watch: false, auto_scan_interval: null }
		}
	});
});

test('automatic scans and watching are set under 高级 and shown on the card', async ({ page }, testInfo) => {
	await openSettings(page);
	// The config sample scans daily and is being watched.
	await expect(row(page, '本地作品')).toContainText('正在监视');
	await expect(row(page, '本地作品')).toContainText('下次自动扫描');

	await startAdding(page);
	await field(page, '名称').fill('Watched');
	await field(page, '路径').fill('/srv/media/watched');
	await openAdvanced(page);
	const interval = field(page, '自动扫描间隔');
	await expect(interval.getByRole('option', { name: '每 5 分钟' })).toHaveCount(1);
	await expect(sheet(page)).toContainText('本地媒体库最短每 5 分钟一次');
	await interval.selectOption({ label: '每 1 小时' });
	await sheet(page).getByLabel('监视文件夹变化').check();
	await shot(page, testInfo, 'auto-scan');

	// OpenList's minimum is 30 minutes, so its list starts there.
	await sheet(page).getByRole('radio', { name: 'OpenList' }).click();
	await expect(interval.getByRole('option', { name: '每 5 分钟' })).toHaveCount(0);
	await expect(sheet(page).getByLabel('监视文件夹变化')).toHaveCount(0);
	await sheet(page).getByRole('radio', { name: '本地文件夹' }).click();

	await sheet(page).getByRole('button', { name: '添加' }).click();
	await expect(sheet(page)).toBeHidden();
	await expect(row(page, 'Watched')).toContainText('正在监视');
	await expect(row(page, 'Watched')).toContainText('下次自动扫描');
	expect((await rootRequests(page)).at(-1)!.body.settings).toEqual({
		type: 'local',
		path: '/srv/media/watched',
		watch: true,
		auto_scan_interval: 3600
	});

	// Turning both off again sends them off, since PUT replaces every setting.
	await page.getByRole('button', { name: '编辑 Watched' }).click();
	await openAdvanced(page);
	await expect(field(page, '自动扫描间隔')).toHaveValue(/./);
	await field(page, '自动扫描间隔').selectOption({ label: '关闭' });
	await sheet(page).getByLabel('监视文件夹变化').uncheck();
	await sheet(page).getByRole('button', { name: '保存' }).click();
	await expect(sheet(page)).toBeHidden();
	await expect(row(page, 'Watched')).not.toContainText('正在监视');
	await expect(row(page, 'Watched')).not.toContainText('下次自动扫描');
	expect((await rootRequests(page)).at(-1)!.body.settings).toMatchObject({ watch: false, auto_scan_interval: null });
});

test('a short local interval is refused after switching to OpenList', async ({ page }) => {
	await openSettings(page);
	await startAdding(page);
	await openAdvanced(page);
	await field(page, '自动扫描间隔').selectOption({ label: '每 5 分钟' });
	await sheet(page).getByRole('radio', { name: 'OpenList' }).click();
	await field(page, '名称').fill('网盘');
	await field(page, '地址').fill('https://openlist.example.lan');
	await field(page, '用户名').fill('u');
	await field(page, '密码').fill('p');
	await sheet(page).getByRole('button', { name: '添加' }).click();
	await expect(sheet(page)).toContainText('OpenList媒体库最短每 30 分钟扫描一次');
	expect(await rootRequests(page)).toHaveLength(0);
});

test('a failed OpenList check can be saved anyway', async ({ page }, testInfo) => {
	await openSettings(page);
	await startAdding(page);
	await sheet(page).getByRole('radio', { name: 'OpenList' }).click();
	await field(page, '名称').fill('网盘');
	await field(page, '地址').fill('https://unreachable.example.lan');
	await field(page, '目录').fill('/115/新');
	await field(page, '用户名').fill('kikoeru');
	await field(page, '密码').fill('secret');
	await openAdvanced(page);
	// No ASCII in the name, so the suggestion falls back.
	await expect(field(page, 'ID')).toHaveValue('library');
	await field(page, '同时列目录的请求数').fill('2');
	await sheet(page).getByRole('button', { name: '添加' }).click();

	const alert = sheet(page).getByRole('alert');
	await expect(alert).toContainText('连接检查没有通过：连不上 OpenList https://unreachable.example.lan：连接超时');
	// The password is gone from the form once it was sent.
	await expect(field(page, '密码')).toHaveValue('');
	await shot(page, testInfo, 'check-failed');
	await alert.getByRole('button', { name: '仍然保存' }).click();
	await expect(sheet(page)).toBeHidden();
	await expect(row(page, '网盘')).toContainText('OpenList');

	const [first, second] = (await rootRequests(page)).slice(-2);
	expect(first!.body).not.toHaveProperty('skip_check');
	expect(second!.body).toMatchObject({ id: 'library', skip_check: true });
	expect(second!.body.settings).toEqual({
		type: 'openlist',
		url: 'https://unreachable.example.lan',
		base_path: '/115/新',
		username: 'kikoeru',
		password: '<redacted>',
		list_concurrency: 2,
		link_rate: null,
		auto_scan_interval: null
	});
});

test('the form is locked while a save is out, so "仍然保存" sends what it shows', async ({ page }) => {
	await openSettings(page);
	await page.evaluate(() => {
		(globalThis as { __kikoeruMockHoldMs?: Record<string, number> }).__kikoeruMockHoldMs = { 'POST /admin/roots': 1500 };
	});
	await startAdding(page);
	await sheet(page).getByRole('radio', { name: 'OpenList' }).click();
	await field(page, '名称').fill('慢');
	await field(page, '地址').fill('https://unreachable.example.lan');
	await field(page, '用户名').fill('u');
	await field(page, '密码').fill('p');
	await sheet(page).getByRole('button', { name: '添加' }).click();
	await expect(field(page, '地址')).toBeDisabled();
	await expect(sheet(page).getByRole('radio', { name: '本地文件夹' })).toBeDisabled();
	await expect(sheet(page).getByRole('alert')).toContainText('连接检查没有通过');
	await expect(field(page, '地址')).toBeEnabled();
	// An edit after the failure withdraws the offer to save the old request.
	await field(page, '地址').fill('https://unreachable-2.example.lan');
	await expect(sheet(page).getByRole('button', { name: '仍然保存' })).toHaveCount(0);
});

test('adds a WebDAV share, saving it anyway after a failed PROPFIND', async ({ page }, testInfo) => {
	await openSettings(page);
	await startAdding(page);
	await sheet(page).getByRole('radio', { name: 'WebDAV' }).click();
	await expect(sheet(page).getByRole('radio', { name: 'WebDAV' })).toHaveAttribute('aria-checked', 'true');
	await field(page, '名称').fill('Home NAS');
	await expect(field(page, '地址')).toHaveAttribute('placeholder', 'https://nas.example.lan/dav');
	await field(page, '地址').fill('https://unreachable.nas.lan/dav');
	await field(page, '目录').fill('/音声');
	await field(page, '用户名').fill('kikoeru');
	await field(page, '密码').fill('secret');
	await openAdvanced(page);
	// WebDAV has no direct links and no watcher, and shares the 30 minute floor.
	await expect(sheet(page).getByLabel('每秒取直链的次数')).toHaveCount(0);
	await expect(sheet(page).getByLabel('监视文件夹变化')).toHaveCount(0);
	await expect(field(page, '自动扫描间隔').getByRole('option', { name: '每 15 分钟' })).toHaveCount(0);
	await expect(sheet(page)).toContainText('WebDAV媒体库最短每 30 分钟一次');
	await field(page, '自动扫描间隔').selectOption({ label: '每 6 小时' });
	await field(page, '同时列目录的请求数').fill('3');
	await shot(page, testInfo, 'add-webdav');
	await sheet(page).getByRole('button', { name: '添加' }).click();

	const alert = sheet(page).getByRole('alert');
	await expect(alert).toContainText('连接检查没有通过：连不上 WebDAV https://unreachable.nas.lan/dav：连接超时');
	await expect(alert).toContainText('等 WebDAV 能连上之后再扫描');
	await expect(field(page, '密码')).toHaveValue('');
	await alert.getByRole('button', { name: '仍然保存' }).click();
	await expect(sheet(page)).toBeHidden();
	await expect(row(page, 'Home NAS')).toContainText('WebDAV');
	await expect(row(page, 'Home NAS')).toContainText('下次自动扫描');

	const [first, second] = (await rootRequests(page)).slice(-2);
	expect(first!.body).not.toHaveProperty('skip_check');
	expect(second!.body).toMatchObject({ id: 'home-nas', skip_check: true });
	expect(second!.body.settings).toEqual({
		type: 'webdav',
		url: 'https://unreachable.nas.lan/dav',
		base_path: '/音声',
		username: 'kikoeru',
		password: '<redacted>',
		list_concurrency: 3,
		auto_scan_interval: 21_600
	});
});

test('editing a WebDAV share keeps its password unless the server or the kind changes', async ({ page }) => {
	await openSettings(page);
	await page.getByRole('button', { name: '编辑 NAS' }).click();
	await expect(sheet(page).getByRole('heading', { name: '编辑媒体库' })).toBeVisible();
	await expect(sheet(page).getByRole('radio', { name: 'WebDAV' })).toHaveAttribute('aria-checked', 'true');
	await expect(sheet(page)).toContainText('已设置密码');
	await expect(field(page, '地址')).toHaveValue('https://nas.example.lan/dav');

	// The same host keeps the stored password.
	await field(page, '目录').fill('/音声/新');
	await sheet(page).getByRole('button', { name: '保存' }).click();
	await expect(sheet(page)).toBeHidden();
	let put = (await rootRequests(page)).at(-1)!;
	expect(put.path).toBe('/admin/roots/nas');
	expect(put.body.settings).toMatchObject({ type: 'webdav', base_path: '/音声/新', password: null });
	expect(put.body.settings).not.toHaveProperty('link_rate');

	// Another host, or OpenList on the same host, needs the password again.
	await page.getByRole('button', { name: '编辑 NAS' }).click();
	await field(page, '地址').fill('https://nas2.example.lan/dav');
	await expect(sheet(page)).toContainText('地址的协议、主机或端口变了，请重新填写密码');
	await field(page, '地址').fill('https://nas.example.lan/dav');
	await sheet(page).getByRole('radio', { name: 'OpenList' }).click();
	await expect(sheet(page)).toContainText('换成 OpenList 之后要重新填写密码');
	const before = (await rootRequests(page)).length;
	await sheet(page).getByRole('button', { name: '保存' }).click();
	await expect(field(page, '密码')).toHaveAttribute('aria-invalid', 'true');
	expect(await rootRequests(page)).toHaveLength(before);

	await field(page, '密码').fill('openlist secret');
	await sheet(page).getByRole('button', { name: '保存' }).click();
	await expect(sheet(page)).toBeHidden();
	put = (await rootRequests(page)).at(-1)!;
	expect(put.body.settings).toMatchObject({ type: 'openlist', password: '<redacted>' });
	await expect(row(page, 'NAS')).toContainText('OpenList');
});

test('editing keeps the stored password when the field stays empty', async ({ page }, testInfo) => {
	await openSettings(page);
	await page.getByRole('button', { name: '编辑 云端作品' }).click();
	await expect(sheet(page).getByRole('heading', { name: '编辑媒体库' })).toBeVisible();
	await expect(sheet(page)).toContainText('已设置密码');
	await expect(field(page, '密码')).toHaveValue('');
	await expect(field(page, '地址')).toHaveValue('https://openlist.example.lan');
	await openAdvanced(page);
	await expect(field(page, 'ID')).toBeDisabled();
	await shot(page, testInfo, 'edit');

	await field(page, '名称').fill('云端作品 新');
	// Another path on the same server still uses the stored password.
	await field(page, '地址').fill('https://openlist.example.lan:443/');
	await sheet(page).getByRole('button', { name: '保存' }).click();
	await expect(sheet(page)).toBeHidden();
	await expect(row(page, '云端作品 新')).toBeVisible();
	const put = (await rootRequests(page)).at(-1)!;
	expect(put.method).toBe('PUT');
	expect(put.path).toBe('/admin/roots/cloud');
	expect(put.body).not.toHaveProperty('id');
	expect(put.body.settings).toMatchObject({ type: 'openlist', password: null });
});

test('moving to another host needs a new password before saving', async ({ page }) => {
	await openSettings(page);
	await page.getByRole('button', { name: '编辑 云端作品' }).click();
	await field(page, '地址').fill('https://new-host.example.lan');
	await expect(sheet(page)).toContainText('地址的协议、主机或端口变了，请重新填写密码');
	await sheet(page).getByRole('button', { name: '保存' }).click();
	await expect(field(page, '密码')).toHaveAttribute('aria-invalid', 'true');
	await expect(sheet(page)).toBeVisible();
	expect((await rootRequests(page)).filter((r) => r.method === 'PUT')).toHaveLength(0);

	await field(page, '密码').fill('new secret');
	await sheet(page).getByRole('button', { name: '保存' }).click();
	await expect(sheet(page)).toBeHidden();
	const put = (await rootRequests(page)).at(-1)!;
	expect(put.body.settings).toMatchObject({ url: 'https://new-host.example.lan', password: '<redacted>' });
});

test('a config root is shown read-only with where to change it', async ({ page }, testInfo) => {
	await openSettings(page);
	await expect(row(page, '本地作品')).toContainText('由配置文件管理');
	await expect(row(page, '云端作品')).not.toContainText('由配置文件管理');
	await page.getByRole('button', { name: '查看 本地作品' }).click();
	const details = sheet(page);
	await expect(details.getByRole('heading', { name: '媒体库详情' })).toBeVisible();
	await expect(details).toContainText('config.toml');
	await expect(details).toContainText('/srv/media/works');
	await expect(details).toContainText('每 1 天');
	await expect(details).toContainText('开启，正在监视');
	await expect(details.getByRole('textbox')).toHaveCount(0);
	await expect(details.getByRole('button', { name: '删除媒体库' })).toHaveCount(0);
	await expect(details.getByRole('button', { name: '保存' })).toHaveCount(0);
	await shot(page, testInfo, 'read-only');
	await details.getByRole('button', { name: '完成' }).click();
	await expect(details).toBeHidden();
});

test('delete asks first, and waits for a running scan before it goes through', async ({ page }, testInfo) => {
	await openSettings(page);
	await page.getByRole('button', { name: '扫描 云端作品' }).click();
	await page.getByRole('button', { name: '编辑 云端作品' }).click();
	await sheet(page).getByRole('button', { name: '删除媒体库' }).click();

	const confirm = page.getByRole('dialog', { name: '删除“云端作品”？' });
	await expect(confirm).toContainText('标记为缺失');
	await expect(confirm).toContainText('缺失的作品和位置');
	await shot(page, testInfo, 'delete-confirm');
	// Cancelling sends nothing.
	await confirm.getByRole('button', { name: '取消' }).click();
	await expect(rows(page)).toHaveCount(3);

	await sheet(page).getByRole('button', { name: '删除媒体库' }).click();
	await confirm.getByRole('button', { name: '删除' }).click();
	const alert = sheet(page).getByRole('alert');
	await expect(alert).toContainText('正在扫描或排队扫描，现在不能删除');
	await alert.getByRole('button', { name: '扫描结束后删除' }).click();
	await expect(alert).toContainText('正在等扫描结束');

	// Mock scans take about 8 seconds.
	await expect(sheet(page)).toBeHidden({ timeout: 20_000 });
	await expect(rows(page)).toHaveCount(2);
	await expect(page.getByRole('list', { name: '缺失的作品' })).toContainText('示例作品二');
});

test('the desktop shell fills the path from the folder dialog', async ({ page }) => {
	// What Tauri injects into the local page, with the shell's two commands.
	await page.addInitScript(() => {
		const w = window as unknown as Record<string, unknown>;
		w.isTauri = true;
		w.__TAURI_INTERNALS__ = {
			invoke: async (command: string) => {
				if (command === 'desktop_state') return { mode: 'local', server_url: null, last_error: null };
				if (command === 'pick_folder') return '/Users/me/Music/ASMR';
				throw `不支持的命令 ${command}`;
			}
		};
	});
	await openSettings(page);
	await startAdding(page);
	await field(page, '名称').fill('ASMR');
	await sheet(page).getByRole('button', { name: '选择文件夹…' }).click();
	await expect(field(page, '路径')).toHaveValue('/Users/me/Music/ASMR');
	await openAdvanced(page);
	await expect(sheet(page).getByLabel('监视文件夹变化')).toBeChecked();
	await sheet(page).getByRole('button', { name: '添加' }).click();
	await expect(row(page, 'ASMR')).toBeVisible();
});

test('a shell without pick_folder explains it and hides the button', async ({ page }) => {
	await page.addInitScript(() => {
		const w = window as unknown as Record<string, unknown>;
		w.isTauri = true;
		w.__TAURI_INTERNALS__ = {
			invoke: async (command: string) => {
				if (command === 'desktop_state') return { mode: 'local', server_url: null, last_error: null };
				throw `Command ${command} not found`;
			}
		};
	});
	await openSettings(page);
	await startAdding(page);
	await sheet(page).getByRole('button', { name: '选择文件夹…' }).click();
	await expect(sheet(page).getByRole('alert')).toContainText('请直接填写路径');
	await expect(sheet(page).getByRole('button', { name: '选择文件夹…' })).toHaveCount(0);
	await sheet(page).getByRole('button', { name: '取消' }).click();
	await startAdding(page);
	await expect(sheet(page).getByRole('button', { name: '选择文件夹…' })).toHaveCount(0);
	// It is still the local page, so local folders are still watched by default.
	await openAdvanced(page);
	await expect(sheet(page).getByLabel('监视文件夹变化')).toBeChecked();
});

test('a browser has no folder dialog button', async ({ page }) => {
	await openSettings(page);
	await startAdding(page);
	await expect(field(page, '路径')).toBeVisible();
	await expect(sheet(page).getByRole('button', { name: '选择文件夹…' })).toHaveCount(0);
	await openAdvanced(page);
	await expect(sheet(page).getByLabel('监视文件夹变化')).not.toBeChecked();
});
