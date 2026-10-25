import { expect, test, type Page } from '@playwright/test';
import type { Page as ApiPage, ScanResponse, WorkDetail, WorkSummary } from '../src/lib/api/types';
import { getJson, login, waitForTask } from './api';
import { ADMIN, METADATA_BASE_URL } from './env';
import { WEBDAV_PASSWORD, WEBDAV_USER, webdav } from './fake-webdav';
import { WORK_TRACK, addUiRootFolder, addWebDavWork } from './metadata-library';

// A library added in the settings page against the real server: the roots
// API stores it in the database, a scan finds its work, and deleting it
// marks the work missing. It runs on the metadata server, whose other roots
// live in config.toml, so the page also shows the read-only badge there.
test.use({ baseURL: METADATA_BASE_URL });

test('a library added in settings is scanned, deleted and its work listed as missing', async ({ page, request }, testInfo) => {
	const project = testInfo.project.name;
	const { path, code } = addUiRootFolder(project);
	const id = `ui-${project}`;
	const name = `${project} 界面添加`;

	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill(ADMIN.name);
	await page.getByLabel('密码').fill(ADMIN.password);
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
	await page.goto('/settings');
	const rows = page.getByRole('list', { name: '媒体库状态' }).getByRole('listitem');
	await expect(rows.first()).toContainText('由配置文件管理');

	await page.getByRole('button', { name: '添加媒体库' }).click();
	const sheet = page.getByRole('dialog', { name: '添加媒体库' });
	await sheet.getByLabel('名称', { exact: true }).fill(name);
	await sheet.getByLabel('路径', { exact: true }).fill(path);
	await sheet.getByText('高级', { exact: true }).click();
	await sheet.getByLabel('ID', { exact: true }).fill(id);
	const created = page.waitForResponse((r) => r.url().endsWith('/api/admin/roots') && r.request().method() === 'POST');
	await sheet.getByRole('button', { name: '添加' }).click();
	expect((await created).status()).toBe(201);
	await expect(sheet).toBeHidden();
	const row = rows.filter({ hasText: name });
	await expect(row).toContainText('还没扫描过');
	await expect(row).not.toContainText('由配置文件管理');

	const started = page.waitForResponse((r) => r.url().endsWith('/api/admin/scan') && r.request().method() === 'POST');
	await page.getByRole('button', { name: `扫描 ${name}`, exact: true }).click();
	const { task_ids } = (await (await started).json()) as ScanResponse;
	const headers = await login(request, ADMIN);
	expect((await waitForTask(request, task_ids[0]!, headers)).state).toBe('done');
	await expect(row).toContainText('1 部', { timeout: 10_000 });

	await page.getByRole('button', { name: `编辑 ${name}` }).click();
	const edit = page.getByRole('dialog', { name: '编辑媒体库' });
	await edit.getByRole('button', { name: '删除媒体库' }).click();
	const deleted = page.waitForResponse((r) => r.url().endsWith(`/api/admin/roots/${id}`) && r.request().method() === 'DELETE');
	await page.getByRole('dialog', { name: `删除“${name}”？` }).getByRole('button', { name: '删除' }).click();
	expect((await deleted).status()).toBe(204);
	await expect(edit).toBeHidden();
	await expect(row).toHaveCount(0);
	await expect(page.getByRole('list', { name: '缺失的作品' })).toContainText(code);

	const roots = await request.get('/api/admin/roots', { headers });
	expect(((await roots.json()) as { id: string }[]).map((r) => r.id)).not.toContain(id);
});

async function signIn(page: Page) {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill(ADMIN.name);
	await page.getByLabel('密码').fill(ADMIN.password);
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

test('a WebDAV share added in settings is checked, scanned, streamed and deleted', async ({ page, request }, testInfo) => {
	const project = testInfo.project.name;
	const { basePath, code } = addWebDavWork(project);
	const id = `dav-${project}`;
	const name = `${project} 网络共享`;
	const headers = await login(request, ADMIN);

	await signIn(page);
	await page.goto('/settings');
	const rows = page.getByRole('list', { name: '媒体库状态' }).getByRole('listitem');
	await page.getByRole('button', { name: '添加媒体库' }).click();
	const sheet = page.getByRole('dialog', { name: '添加媒体库' });
	const field = (label: string) => sheet.getByLabel(label, { exact: true });
	await sheet.getByRole('radio', { name: 'WebDAV' }).click();
	await field('名称').fill(name);
	await field('地址').fill(webdav.url());
	await field('目录').fill(basePath);
	await field('用户名').fill(WEBDAV_USER);
	await field('密码').fill('not the password');
	await sheet.getByText('高级', { exact: true }).click();
	await field('ID').fill(id);

	// The server's Depth 0 PROPFIND is refused, so nothing is saved and the reason shows.
	const refused = page.waitForResponse((r) => r.url().endsWith('/api/admin/roots') && r.request().method() === 'POST');
	await sheet.getByRole('button', { name: '添加' }).click();
	expect([502, 503]).toContain((await refused).status());
	await expect(sheet.getByRole('alert')).toContainText('连接检查没有通过');
	await expect(sheet.getByRole('button', { name: '仍然保存' })).toBeVisible();
	const checks = (await webdav.state()).requests.filter((r) => r === `PROPFIND ${basePath} 0`);
	expect(checks.length).toBeGreaterThan(0);

	// With the right password the check passes and the share is saved.
	await field('密码').fill(WEBDAV_PASSWORD);
	const created = page.waitForResponse((r) => r.url().endsWith('/api/admin/roots') && r.request().method() === 'POST');
	await sheet.getByRole('button', { name: '添加' }).click();
	expect((await created).status()).toBe(201);
	await expect(sheet).toBeHidden();
	const row = rows.filter({ hasText: name });
	await expect(row).toContainText('WebDAV');

	const started = page.waitForResponse((r) => r.url().endsWith('/api/admin/scan') && r.request().method() === 'POST');
	await page.getByRole('button', { name: `扫描 ${name}`, exact: true }).click();
	const { task_ids } = (await (await started).json()) as ScanResponse;
	expect((await waitForTask(request, task_ids[0]!, headers)).state).toBe('done');
	await expect(row).toContainText('1 部', { timeout: 10_000 });
	expect((await webdav.state()).requests).toContain(`PROPFIND ${basePath} 1`);

	// The track streams through the server with a Range request to the share.
	const works = await getJson<ApiPage<WorkSummary>>(request, `/api/works?page=1&root=${id}`, headers);
	const work = works.items.find((w) => w.code === code);
	expect(work, code).toBeDefined();
	const detail = await getJson<WorkDetail>(request, `/api/works/${work!.id}`, headers);
	const track = detail.tracks.find((t) => t.path.endsWith(WORK_TRACK.file));
	expect(track).toBeDefined();
	const chunk = await request.get(`/api/tracks/${track!.id}/stream`, { headers: { ...headers, Range: 'bytes=0-43' } });
	expect(chunk.status()).toBe(206);
	expect((await chunk.body()).subarray(0, 4).toString()).toBe('RIFF');
	expect((await webdav.state()).requests.some((r) => r.startsWith(`GET ${basePath}/${code}/${WORK_TRACK.file} bytes=`))).toBe(true);

	// A rename keeps the stored password, since the server did not change.
	await page.getByRole('button', { name: `编辑 ${name}` }).click();
	const edit = page.getByRole('dialog', { name: '编辑媒体库' });
	await expect(edit).toContainText('已设置密码');
	await edit.getByLabel('名称', { exact: true }).fill(`${name} 2`);
	const updated = page.waitForResponse((r) => r.url().endsWith(`/api/admin/roots/${id}`) && r.request().method() === 'PUT');
	await edit.getByRole('button', { name: '保存' }).click();
	expect((await updated).status()).toBe(200);
	await expect(edit).toBeHidden();
	const renamed = rows.filter({ hasText: `${name} 2` });
	await expect(renamed).toBeVisible();

	await page.getByRole('button', { name: `编辑 ${name} 2` }).click();
	await edit.getByRole('button', { name: '删除媒体库' }).click();
	const deleted = page.waitForResponse((r) => r.url().endsWith(`/api/admin/roots/${id}`) && r.request().method() === 'DELETE');
	await page.getByRole('dialog', { name: `删除“${name} 2”？` }).getByRole('button', { name: '删除' }).click();
	expect((await deleted).status()).toBe(204);
	await expect(renamed).toHaveCount(0);
	await expect(page.getByRole('list', { name: '缺失的作品' })).toContainText(code);
});
