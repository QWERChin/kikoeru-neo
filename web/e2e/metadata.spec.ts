import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type APIRequestContext, type Page, type TestInfo } from '@playwright/test';
import type { Page as ApiPage, ScanResponse, TaskSnapshot, WorkDetail, WorkSummary } from '../src/lib/api/types';
import { getJson, login, waitForTask } from './api';
import { ADMIN, METADATA_BASE_URL } from './env';
import {
	DLSITE_CIRCLE,
	DLSITE_COVER,
	DLSITE_RATING,
	DLSITE_RELEASE,
	DLSITE_TAGS,
	DLSITE_VAS,
	dlsite,
	dlsiteTitle
} from './fake-dlsite';
import { WORK_TRACK, addWork, metadataRoot, type Scenario } from './metadata-library';

// The M4 completion conditions in the browser, on the metadata server that
// scrapes the fake DLsite (fake-dlsite.ts). Each test copies a new work
// into a root of its own and scans it, as an admin would after adding files.
test.use({ baseURL: METADATA_BASE_URL });

type Headers = Record<string, string>;

test.beforeEach(async () => {
	await dlsite.set({ mode: 'up', edition: 1 });
});

// A failed test must not leave the fake holding requests for the next one.
test.afterEach(async () => {
	await dlsite.set({ mode: 'up', edition: 1 });
});

const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

async function signIn(page: Page) {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill(ADMIN.name);
	await page.getByLabel('密码').fill(ADMIN.password);
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

async function shot(page: Page, testInfo: TestInfo, name: string) {
	const dir = process.env.KIKOERU_E2E_SCREENSHOTS;
	if (!dir) return;
	mkdirSync(dir, { recursive: true });
	await page.screenshot({ path: join(dir, `m4-e2e-${name}-${testInfo.project.name}.png`), fullPage: true });
}

function root(testInfo: TestInfo, scenario: Scenario) {
	return metadataRoot(testInfo.project.name, scenario);
}

async function openSettings(page: Page) {
	// The roots list shows once the page has read the roots' status.
	const roots = page.waitForResponse(
		(r) => new URL(r.url()).pathname === '/api/admin/roots' && r.request().method() === 'GET'
	);
	await page.goto('/settings');
	expect((await roots).status()).toBe(200);
	await expect(page.getByRole('heading', { name: '设置', level: 1 })).toBeVisible();
	await expect(page.getByRole('list', { name: '媒体库状态' })).toBeVisible();
}

/** The progress card of a followed scan, by the label the page gave it. */
function scanCard(page: Page, label: string) {
	return page.getByRole('list', { name: '扫描进度' }).getByRole('listitem', { name: label, exact: true });
}

/** Clicks the root's scan button and returns the task the server started. */
async function scanFromSettings(page: Page, display: string): Promise<number> {
	const started = page.waitForResponse(
		(r) => r.url().endsWith('/api/admin/scan') && r.request().method() === 'POST'
	);
	await page.getByRole('button', { name: `扫描 ${display}`, exact: true }).click();
	const response = await started;
	expect(response.status()).toBe(202);
	const { task_ids } = (await response.json()) as ScanResponse;
	expect(task_ids).toHaveLength(1);
	return task_ids[0]!;
}

async function workId(request: APIRequestContext, headers: Headers, code: string): Promise<number> {
	const page = await getJson<ApiPage<WorkSummary>>(request, '/api/works?page=1', headers);
	const work = page.items.find((w) => w.code === code);
	expect(work, code).toBeDefined();
	return work!.id;
}

/**
 * The detail page's next read of a work, any work when `id` is left out.
 * The page shows a work only after this answer, which includes booting the
 * app after a navigation. On a loaded machine that alone can outlast an
 * assertion's 5 seconds, so tests wait for the read and then check what it
 * rendered.
 */
function detailRead(page: Page, id?: number) {
	return page.waitForResponse((r) => {
		const path = new URL(r.url()).pathname;
		const match = id === undefined ? /^\/api\/works\/\d+$/.test(path) : path === `/api/works/${id}`;
		return match && r.request().method() === 'GET';
	});
}

/** Opens a work's detail page and waits until the page has read it. */
async function openWork(page: Page, id: number) {
	const read = detailRead(page, id);
	await page.goto(`/works/${id}`);
	expect((await read).status()).toBe(200);
}

const detail = (request: APIRequestContext, headers: Headers, id: number) =>
	getJson<WorkDetail>(request, `/api/works/${id}`, headers);

async function scanTasks(request: APIRequestContext, headers: Headers): Promise<number> {
	const page = await getJson<ApiPage<TaskSnapshot>>(request, '/api/tasks?page_size=100', headers);
	return page.items.filter((t) => t.kind === 'scan').length;
}

/** The cover the detail page shows, as the browser decoded it. */
function coverImage(page: Page) {
	return page.locator('article img[src*="/cover?"]').first();
}
/** Tag names in the order the page shows them, sorted so the test does not depend on it. */
const tagNames = async (page: Page) => (await page.getByRole('list', { name: '标签' }).getByRole('listitem').allTextContents()).sort();
const naturalSize = (page: Page) =>
	coverImage(page).evaluate((img: HTMLImageElement) => (img.complete ? [img.naturalWidth, img.naturalHeight] : null));

/** Requests the fake DLsite saw for one code. */
async function dlsiteRequests(code: string) {
	return (await dlsite.state()).requests.filter((r) => r.code === code);
}

/** Waits until a scan's metadata fetch is parked in the fake. */
async function untilHeld(code: string) {
	await expect
		.poll(async () => {
			const state = await dlsite.state();
			return state.held > 0 && state.requests.some((r) => r.code === code);
		})
		.toBe(true);
}

test('a new work scanned from the settings page gets its metadata and cover', async ({ page }, testInfo) => {
	const { display_name } = root(testInfo, 'new');
	const code = addWork(testInfo.project.name, 'new');
	const title = dlsiteTitle(code);

	await signIn(page);
	await openSettings(page);
	await scanFromSettings(page, display_name);
	const card = scanCard(page, display_name);
	await expect(card).toContainText('已完成', { timeout: 30_000 });
	await expect(card).toContainText('扫描已完成');

	await page.getByRole('navigation', { name: '主导航' }).getByRole('link', { name: '资料库', exact: true }).click();
	const read = detailRead(page);
	await page.getByRole('link', { name: new RegExp(escape(title)) }).first().click();
	expect((await read).status()).toBe(200);
	await expect(page.getByRole('heading', { name: title, level: 1 })).toBeVisible();
	await expect(page.getByText(`${code} · ${DLSITE_RELEASE}`)).toBeVisible();
	await expect(page.getByText(DLSITE_CIRCLE, { exact: true })).toBeVisible();
	for (const va of DLSITE_VAS) await expect(page.locator('dd').filter({ hasText: va })).toBeVisible();
	await expect.poll(() => tagNames(page)).toEqual([...DLSITE_TAGS].sort());
	await expect(page.getByText(`（${DLSITE_RATING.count} 人评价）`)).toBeVisible();
	await expect.poll(() => naturalSize(page)).toEqual([DLSITE_COVER.width, DLSITE_COVER.height]);
	// The media token went in the cover cookie, so the URL stays cacheable.
	const src = await coverImage(page).getAttribute('src');
	expect(src).not.toContain('token=');
	expect((await dlsiteRequests(code)).map((r) => r.kind).sort()).toEqual(['cover', 'info', 'product']);
	await shot(page, testInfo, 'fetched');

	// After signing out, the browser still holds the cover, but it has to
	// revalidate it, and without the cookie the server refuses.
	await openSettings(page);
	await page.getByRole('button', { name: '退出登录' }).click();
	await expect(page).toHaveURL(/\/login$/);
	// A new document, as someone opening the browser later would get. The
	// old one may reuse an image it already decoded without asking anyone.
	await page.goto('/login');
	const shown = await page.evaluate(
		(url) =>
			new Promise<boolean>((resolve) => {
				const img = new Image();
				img.onload = () => resolve(true);
				img.onerror = () => resolve(false);
				img.src = url;
			}),
		src!
	);
	expect(shown).toBe(false);
});

test('during an outage a new work is added and plays, and a refresh fills in only its metadata', async ({
	page,
	request
}, testInfo) => {
	test.setTimeout(90_000);
	const admin = await login(request, ADMIN);
	const { display_name } = root(testInfo, 'outage');
	await dlsite.set({ mode: 'down' });
	const code = addWork(testInfo.project.name, 'outage');

	await signIn(page);
	await openSettings(page);
	const scan = await scanFromSettings(page, display_name);
	// Every DLsite request is retried with backoff before the scan gives up on it.
	await expect(scanCard(page, display_name)).toContainText('已完成', { timeout: 60_000 });
	expect((await waitForTask(request, scan, admin)).state).toBe('done');
	expect((await dlsiteRequests(code)).length).toBeGreaterThan(0);

	const id = await workId(request, admin, code);
	const before = await detail(request, admin, id);
	await openWork(page, id);
	// The directory name stands in for the title.
	await expect(page.getByRole('heading', { name: code, level: 1 })).toBeVisible();
	await expect(page.getByText('未知社团')).toBeVisible();
	await expect(coverImage(page)).toHaveCount(0);

	await page.getByRole('list', { name: '曲目' }).getByRole('button', { name: new RegExp(`^${escape(WORK_TRACK.file)}`) }).click();
	const mini = page.getByRole('region', { name: '迷你播放条' });
	await expect(mini.getByRole('button', { name: '暂停' })).toBeVisible();
	await expect
		.poll(() => page.locator('media-player audio').evaluate((el: HTMLAudioElement) => el.currentTime))
		.toBeGreaterThan(0.5);
	await mini.getByRole('button', { name: '暂停' }).click();

	// DLsite is back. A refresh is a metadata task, not another scan.
	await dlsite.set({ mode: 'up' });
	const scans = await scanTasks(request, admin);
	const refreshed = page.waitForResponse(
		(r) => r.url().includes(`/api/works/${id}/refresh`) && r.request().method() === 'POST'
	);
	// The page reads the work again once it sees the task end, which can
	// happen before the click returns.
	const reread = detailRead(page, id);
	await page.getByRole('button', { name: '重新抓取' }).click();
	const { task_id } = (await (await refreshed).json()) as { task_id: number };
	const task = await waitForTask(request, task_id, admin);
	expect([task.kind, task.state]).toEqual(['metadata', 'done']);
	expect((await reread).status()).toBe(200);
	await expect(page.getByRole('status').filter({ hasText: '重新抓取：已完成' })).toBeVisible();
	await expect(page.getByRole('heading', { name: dlsiteTitle(code), level: 1 })).toBeVisible();
	await expect.poll(() => naturalSize(page)).toEqual([DLSITE_COVER.width, DLSITE_COVER.height]);
	expect(await scanTasks(request, admin)).toBe(scans);
	const after = await detail(request, admin, id);
	expect(after.tracks.map((t) => t.id)).toEqual(before.tracks.map((t) => t.id));
});

test('a manual edit survives a normal refresh and only a forced one replaces it', async ({ page, request }, testInfo) => {
	test.setTimeout(90_000);
	const admin = await login(request, ADMIN);
	const code = addWork(testInfo.project.name, 'edit');
	const scan = await request.post('/api/admin/scan', { headers: admin, data: { root: root(testInfo, 'edit').id } });
	expect(scan.status()).toBe(202);
	const [scanId] = ((await scan.json()) as ScanResponse).task_ids;
	expect((await waitForTask(request, scanId!, admin)).state).toBe('done');
	const id = await workId(request, admin, code);
	// The scan fetched the metadata itself, so the page has nothing to wait for but its own read.
	expect((await detail(request, admin, id)).title).toBe(dlsiteTitle(code));

	await signIn(page);
	await openWork(page, id);
	await expect(page.getByRole('heading', { name: dlsiteTitle(code), level: 1 })).toBeVisible();
	await page.getByRole('button', { name: '编辑元数据' }).click();
	const dialog = page.getByRole('dialog', { name: '编辑元数据' });
	await expect(dialog.getByLabel('标题')).toHaveValue(dlsiteTitle(code));
	await dialog.getByLabel('标题').fill('E2E 手动标题');
	await dialog.getByLabel('社团').fill('E2E 手动社团');
	await dialog.getByLabel('声优').fill('E2E 手动声优');
	await dialog.getByLabel('标签').fill('E2E 手动标签');
	// A save is followed by a fresh read of the work, which shows the edit.
	const saved = page.waitForResponse(
		(r) => new URL(r.url()).pathname === `/api/works/${id}/metadata` && r.request().method() === 'PUT'
	);
	const reread = detailRead(page, id);
	await dialog.getByRole('button', { name: '保存' }).click();
	expect((await saved).ok()).toBe(true);
	expect((await reread).status()).toBe(200);
	await expect(dialog).toBeHidden();
	await expect(page.getByRole('heading', { name: 'E2E 手动标题', level: 1 })).toBeVisible();
	await expect(page.getByRole('status').filter({ hasText: '普通的重新抓取不会覆盖它' })).toBeVisible();

	// DLsite now answers with a new title, so a refresh that replaced the
	// edit could not go unnoticed.
	await dlsite.set({ edition: 2 });
	const asked = (await dlsiteRequests(code)).length;
	const refresh = async (force: boolean) => {
		const started = page.waitForResponse((r) => {
			const url = new URL(r.url());
			return (
				url.pathname === `/api/works/${id}/refresh` &&
				r.request().method() === 'POST' &&
				(url.searchParams.get('force') === 'true') === force
			);
		});
		// The page reloads the work once the task is done.
		const reloaded = detailRead(page, id);
		await page.getByRole('button', { name: '重新抓取' }).click();
		if (force) {
			const confirm = page.getByRole('dialog', { name: '强制重新抓取？' });
			await confirm.getByRole('button', { name: '覆盖并抓取' }).click();
		}
		const { task_id } = (await (await started).json()) as { task_id: number };
		expect((await waitForTask(request, task_id, admin)).state).toBe('done');
		expect((await reloaded).status()).toBe(200);
		await expect(page.getByRole('status').filter({ hasText: '重新抓取：已完成' })).toBeVisible();
	};

	await refresh(false);
	expect((await dlsiteRequests(code)).length).toBeGreaterThan(asked);
	await expect(page.getByRole('heading', { name: 'E2E 手动标题', level: 1 })).toBeVisible();
	const reread2 = detailRead(page, id);
	await page.reload();
	expect((await reread2).status()).toBe(200);
	await expect(page.getByRole('heading', { name: 'E2E 手动标题', level: 1 })).toBeVisible();
	await expect(page.getByText('E2E 手动社团', { exact: true })).toBeVisible();
	await expect(page.getByRole('list', { name: '标签' }).getByRole('listitem')).toHaveText(['E2E 手动标签']);

	await page.getByLabel('强制覆盖手动修改').check();
	await expect(page.getByRole('note')).toContainText('覆盖手动修改过的元数据');
	await refresh(true);
	await expect(page.getByRole('heading', { name: dlsiteTitle(code, 2), level: 1 })).toBeVisible();
	await expect(page.getByText(DLSITE_CIRCLE, { exact: true })).toBeVisible();
	await expect.poll(() => tagNames(page)).toEqual([...DLSITE_TAGS].sort());
	await shot(page, testInfo, 'forced');
});

test('closing the settings page leaves a scan running, and the page follows it again when reopened', async ({
	page,
	context,
	request
}, testInfo) => {
	const admin = await login(request, ADMIN);
	const { display_name } = root(testInfo, 'close');
	await dlsite.set({ mode: 'hold' });
	const code = addWork(testInfo.project.name, 'close');

	await signIn(page);
	await openSettings(page);
	const scan = await scanFromSettings(page, display_name);
	const card = scanCard(page, display_name);
	await expect(card.locator('[aria-current="step"]')).toContainText('抓取元数据');
	await expect(card).toContainText('实时更新');
	await untilHeld(code);
	await page.close();

	// Nothing told the server to stop: the scan still waits on DLsite.
	expect((await getJson<TaskSnapshot>(request, `/api/tasks/${scan}`, admin)).state).toBe('running');
	const reopened = await context.newPage();
	await openSettings(reopened);
	// A new page has no name for a scan it did not start.
	const again = scanCard(reopened, `扫描任务 #${scan}`);
	await expect(again).toContainText('进行中');
	await expect(again.locator('[aria-current="step"]')).toContainText('抓取元数据');
	await expect(again).toContainText('实时更新');
	await shot(reopened, testInfo, 'reopened');

	await dlsite.set({ mode: 'up' });
	await expect(again).toContainText('已完成', { timeout: 30_000 });
	expect((await waitForTask(request, scan, admin)).state).toBe('done');
	const id = await workId(request, admin, code);
	expect((await detail(request, admin, id)).title).toBe(dlsiteTitle(code));
});

test('cancelling a scan from the settings page stops it and leaves the work without metadata', async ({
	page,
	request
}, testInfo) => {
	const admin = await login(request, ADMIN);
	const { display_name } = root(testInfo, 'cancel');
	await dlsite.set({ mode: 'hold' });
	const code = addWork(testInfo.project.name, 'cancel');

	await signIn(page);
	await openSettings(page);
	const scan = await scanFromSettings(page, display_name);
	const card = scanCard(page, display_name);
	await expect(card.locator('[aria-current="step"]')).toContainText('抓取元数据');
	await untilHeld(code);

	await card.getByRole('button', { name: '取消任务' }).click();
	await expect(card).toContainText('已取消');
	await expect(card.getByRole('button', { name: '移除' })).toBeVisible();
	expect((await getJson<TaskSnapshot>(request, `/api/tasks/${scan}`, admin)).state).toBe('cancelled');
	await page.getByRole('region', { name: '任务' }).getByRole('button', { name: '刷新' }).click();
	await expect(
		page.getByRole('list', { name: '任务列表' }).getByRole('listitem').filter({ hasText: `扫描 #${scan}` })
	).toContainText('已取消');

	// The scan added the work before its metadata stage. Releasing DLsite
	// now answers a request nobody waits for any more.
	await dlsite.set({ mode: 'up' });
	const id = await workId(request, admin, code);
	const work = await detail(request, admin, id);
	expect([work.title, work.has_cover]).toEqual([code, false]);
	await openWork(page, id);
	await expect(page.getByRole('heading', { name: code, level: 1 })).toBeVisible();
});

/** The labels of the scan cards, top to bottom. */
function cardLabels(page: Page) {
	return page
		.getByRole('list', { name: '扫描进度' })
		.locator(':scope > li')
		.evaluateAll((cards) => cards.map((card) => card.getAttribute('aria-label')));
}

test('a queued scan moved to the front is listed next and runs before the one it passed', async ({
	page,
	request
}, testInfo) => {
	const admin = await login(request, ADMIN);
	const hold = root(testInfo, 'queueHold').display_name;
	const first = root(testInfo, 'queueFirst').display_name;
	const moved = root(testInfo, 'queueMoved').display_name;
	await dlsite.set({ mode: 'hold' });
	const holdCode = addWork(testInfo.project.name, 'queueHold');
	// The moved scan also waits on DLsite once it runs, so the page can show
	// it running while the scan it passed still waits. The server refuses to
	// scan an empty local root, so the last one gets a work too.
	const movedCode = addWork(testInfo.project.name, 'queueMoved');
	addWork(testInfo.project.name, 'queueFirst');

	await signIn(page);
	await openSettings(page);
	const running = await scanFromSettings(page, hold);
	await expect(scanCard(page, hold).locator('[aria-current="step"]')).toContainText('抓取元数据');
	await untilHeld(holdCode);
	const waiting = await scanFromSettings(page, first);
	const jumping = await scanFromSettings(page, moved);
	await expect(scanCard(page, first)).toContainText('排队中 · 第 1 位');
	await expect(scanCard(page, moved)).toContainText('排队中 · 第 2 位');
	await expect(scanCard(page, first).getByRole('button', { name: '移到最前' })).toHaveCount(0);
	await expect(scanCard(page, hold).getByRole('button', { name: '移到最前' })).toHaveCount(0);
	await expect.poll(() => cardLabels(page)).toEqual([hold, first, moved]);
	await shot(page, testInfo, 'queue');

	await scanCard(page, moved).getByRole('button', { name: '移到最前' }).click();
	await expect(scanCard(page, moved)).toContainText('排队中 · 第 1 位');
	await expect(scanCard(page, first)).toContainText('排队中 · 第 2 位');
	await expect.poll(() => cardLabels(page)).toEqual([hold, moved, first]);
	await expect(scanCard(page, hold)).toContainText('进行中');
	expect((await getJson<TaskSnapshot>(request, `/api/tasks/${jumping}`, admin)).queue_position).toBe(1);
	await shot(page, testInfo, 'queue-moved');

	// Cancelling the running scan frees the slot for the moved one.
	await scanCard(page, hold).getByRole('button', { name: '取消任务' }).click();
	await expect(scanCard(page, hold)).toContainText('已取消');
	await untilHeld(movedCode);
	await expect(scanCard(page, moved)).toContainText('进行中');
	await expect(scanCard(page, first)).toContainText('排队中 · 第 1 位');
	await expect.poll(() => cardLabels(page)).toEqual([moved, first, hold]);

	await dlsite.set({ mode: 'up' });
	await expect(scanCard(page, first)).toContainText('已完成', { timeout: 30_000 });
	expect((await waitForTask(request, running, admin)).state).toBe('cancelled');
	expect((await waitForTask(request, jumping, admin)).state).toBe('done');
	expect((await waitForTask(request, waiting, admin)).state).toBe('done');
});
