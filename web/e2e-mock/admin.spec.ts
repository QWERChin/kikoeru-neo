import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page, type TestInfo } from '@playwright/test';

// The M4 admin pages in mock mode. Mock scans move through their stages in
// about 8 seconds and start background probes (src/lib/api/mock-tasks.ts).
// Every test loads the app afresh, so the mock state starts from the samples.

async function signIn(page: Page, name = 'admin') {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill(name);
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

async function navigate(page: Page, label: string) {
	await page.getByRole('navigation', { name: '主导航' }).getByRole('link', { name: label, exact: true }).click();
	await expect(page.getByRole('heading', { name: label, level: 1 })).toBeVisible();
}

async function openSettings(page: Page) {
	await signIn(page);
	await navigate(page, '设置');
	await expect(page.getByRole('list', { name: '媒体库状态' }).getByRole('listitem')).toHaveCount(3);
}

async function shot(page: Page, testInfo: TestInfo, name: string, target?: ReturnType<Page['locator']>) {
	const dir = process.env.KIKOERU_MOCK_SCREENSHOTS;
	if (!dir) return;
	mkdirSync(dir, { recursive: true });
	await target?.scrollIntoViewIfNeeded();
	await page.screenshot({ path: join(dir, `m4-${name}-${testInfo.project.name}.png`) });
}

// A ListPicker field: its trigger button, and the sheet it opens.
const pickerButton = (page: Page, label: string) => page.getByRole('button', { name: new RegExp(`^${label}`) });
const pickerSheet = (page: Page, label: string) => page.getByRole('dialog', { name: label, exact: true });

async function pick(page: Page, label: string, option: string, search?: string) {
	await pickerButton(page, label).click();
	const sheet = pickerSheet(page, label);
	if (search !== undefined) await sheet.getByRole('combobox').fill(search);
	await sheet.getByRole('option', { name: option, exact: true }).click();
	await expect(sheet).toBeHidden();
}

const progress = (page: Page) => page.getByRole('list', { name: '扫描进度' });
const card = (page: Page, label: string) => progress(page).getByRole('listitem', { name: label, exact: true });
const scanRequests = (page: Page) =>
	page.evaluate(() => (globalThis as { __kikoeruMockScans?: unknown[] }).__kikoeruMockScans ?? []);

test('a scan shows its stages live, and cancel stops it', async ({ page }, testInfo) => {
	await openSettings(page);
	await page.getByRole('button', { name: '扫描 本地作品' }).click();
	const scan = card(page, '本地作品');
	await expect(scan.getByRole('list', { name: '扫描阶段' })).toBeVisible();
	await expect(scan.locator('[aria-current="step"]')).toContainText('同步轨道');
	await expect(scan).toContainText('实时更新');
	await expect(scan.getByRole('progressbar')).toHaveAttribute('aria-valuenow', /\d+/);
	await shot(page, testInfo, 'settings-scan', progress(page));

	await scan.getByRole('button', { name: '取消任务' }).click();
	await expect(scan).toContainText('已取消');
	await expect(scan.getByRole('button', { name: '移除' })).toBeVisible();
	// The task list shows the same end state. The probes the scan queued
	// before the cancel are separate tasks and are not cancelled with it.
	await page.getByRole('region', { name: '任务' }).getByRole('button', { name: '刷新' }).click();
	const id = (await scan.textContent())?.match(/扫描任务 #(\d+)/)?.[1];
	await expect(
		page.getByRole('list', { name: '任务列表' }).getByRole('listitem').filter({ hasText: `扫描 #${id}` })
	).toContainText('已取消');
	expect(await scanRequests(page)).toEqual([{ root: 'local' }]);
});

test('a library that is being scanned shows the scan instead of its last scan time', async ({ page }) => {
	await openSettings(page);
	// 云端作品 has never been scanned in the samples.
	const row = page.getByRole('list', { name: '媒体库状态' }).getByRole('listitem').filter({ hasText: '云端作品' });
	await expect(row).toContainText('还没扫描过');
	await page.getByRole('button', { name: '扫描 云端作品' }).click();
	await expect(row).toContainText(/正在扫描 · 同步轨道 \d+\/48/);
	await expect(row).not.toContainText('还没扫描过');
	await expect(row.getByRole('button', { name: '扫描 云端作品' })).toHaveText('扫描中');
	// The other library is not part of this scan.
	const other = page.getByRole('list', { name: '媒体库状态' }).getByRole('listitem').filter({ hasText: '本地作品' });
	await expect(other).toContainText('上次扫描');
	await expect(row).toContainText('上次扫描', { timeout: 15_000 });
});

test('scan completion refreshes library status even with an older response in flight', async ({ page }) => {
	await page.clock.setFixedTime(new Date('2026-10-23T12:00:00Z'));
	await openSettings(page);
	const row = page.getByRole('list', { name: '媒体库状态' }).getByRole('listitem').filter({ hasText: '云端作品' });
	await expect(row).toContainText('48 部');
	await expect(row).toContainText('还没检查');
	await expect(row).toContainText('还没扫描过');
	await page.getByRole('button', { name: '扫描 云端作品' }).click();
	await expect(card(page, '云端作品')).toContainText('实时更新');

	// Hold only the next answer, after the mock has copied the old status.
	const pending = await page.evaluateHandle(() => {
		const hooks = globalThis as { __kikoeruMockWait?: Record<string, Promise<void>> };
		let captured = false;
		let release = () => {};
		const promise = new Promise<void>((resolve) => (release = resolve));
		hooks.__kikoeruMockWait = {
			get ['GET /admin/roots']() {
				captured = true;
				delete hooks.__kikoeruMockWait?.['GET /admin/roots'];
				return promise;
			}
		};
		return { get captured() { return captured; }, release };
	});
	await page.getByRole('region', { name: '媒体库', exact: true }).getByRole('button', { name: '刷新' }).click();
	await expect.poll(() => pending.evaluate((request) => request.captured)).toBe(true);
	const requestsBeforeCompletion = await page.evaluate(
		() => (globalThis as { __kikoeruMockRequests?: Record<string, number> }).__kikoeruMockRequests!['GET /admin/roots']!
	);

	// The scan commits new status before its terminal snapshot is delivered.
	await page.evaluate(async () => {
		const path = '/src/lib/api/mock-roots.ts';
		const { rootsState } = await import(/* @vite-ignore */ path) as typeof import('../src/lib/api/mock-roots');
		const root = rootsState.find((root) => root.id === 'cloud')!;
		root.work_count = 49;
		root.status = 'ok';
	});
	await page.clock.setFixedTime(new Date('2026-10-23T12:01:00Z'));
	await expect(card(page, '云端作品')).toContainText('已完成');
	await expect.poll(() => page.evaluate(
		() => (globalThis as { __kikoeruMockRequests?: Record<string, number> }).__kikoeruMockRequests!['GET /admin/roots']!
	)).toBe(requestsBeforeCompletion + 1);
	await expect(row).toContainText('49 部');
	await expect(row).toContainText('正常');
	await expect(row).toContainText('上次扫描 2026年10月23日');

	await pending.evaluate(async (request) => {
		request.release();
		await new Promise<number>(requestAnimationFrame);
	});
	await expect(row).toContainText('49 部');
	await expect(row).toContainText('正常');
	await expect(row).toContainText('上次扫描 2026年10月23日');
});

test('leaving the page closes the stream but the scan keeps running', async ({ page }) => {
	await openSettings(page);
	await page.getByRole('button', { name: '扫描 云端作品' }).click();
	await expect(card(page, '云端作品')).toContainText('进行中');
	await navigate(page, '资料库');
	await navigate(page, '设置');
	// The activity poll finds the scan again and keeps its name.
	const scan = card(page, '云端作品');
	await expect(scan).toBeVisible();
	await expect(scan).not.toContainText('已取消');
	await expect(scan).toContainText('已完成', { timeout: 15_000 });
});

test('a full scan sends the full flag, and probes are shown apart from the scan', async ({ page }) => {
	await openSettings(page);
	await page.getByLabel('完整扫描（刷新网盘缓存）').check();
	await page.getByRole('button', { name: '完整扫描全部' }).click();
	await expect(card(page, '全部媒体库 1/3 · 完整扫描')).toBeVisible();
	await expect(card(page, '全部媒体库 3/3 · 完整扫描')).toBeVisible();
	expect(await scanRequests(page)).toEqual([{ root: null, full: true }]);

	await expect(card(page, '全部媒体库 1/3 · 完整扫描')).toContainText('已完成', { timeout: 15_000 });
	// The scan is done while its probes still run in the background.
	await expect(page.getByRole('group', { name: '后台探测' })).toContainText('个进行中');
});

test('the task list pages through every state, interrupted included', async ({ page }) => {
	await openSettings(page);
	const list = page.getByRole('list', { name: '任务列表' });
	await expect(list.getByRole('listitem')).toHaveCount(20);
	await expect(page.getByText('第 1 / 3 页 · 共 46 个')).toBeVisible();
	await page.getByRole('navigation', { name: '任务翻页' }).getByRole('button', { name: '下一页' }).click();
	await expect(page.getByText('第 2 / 3 页 · 共 46 个')).toBeVisible();
	await page.getByLabel('按状态筛选').selectOption({ label: '已中断' });
	await expect(page.getByText(/第 1 \/ 1 页/)).toBeVisible();
	await expect(list.getByRole('listitem').first()).toContainText('已中断');
});

test('the task list hides loudness analyses until asked', async ({ page }, testInfo) => {
	await openSettings(page);
	const list = page.getByRole('list', { name: '任务列表' });
	const toggle = page.getByRole('checkbox', { name: '显示响度分析' });
	await expect(toggle).not.toBeChecked();
	await expect(page.getByText('第 1 / 3 页 · 共 46 个')).toBeVisible();
	await expect(list).not.toContainText('响度分析');

	await page.getByRole('navigation', { name: '任务翻页' }).getByRole('button', { name: '下一页' }).click();
	await expect(page.getByText('第 2 / 3 页 · 共 46 个')).toBeVisible();
	await toggle.check();
	// Showing them starts again from the first page, where the newest one is.
	await expect(page.getByText('第 1 / 3 页 · 共 52 个')).toBeVisible();
	await expect(list.getByRole('listitem').filter({ hasText: '响度分析 #50' })).toBeVisible();
	await shot(page, testInfo, 'tasks-analysis', toggle);

	await toggle.uncheck();
	await expect(page.getByText('第 1 / 3 页 · 共 46 个')).toBeVisible();
	await expect(list).not.toContainText('响度分析');
});

test('missing works are cleaned up only after confirming', async ({ page }, testInfo) => {
	await openSettings(page);
	const works = page.getByRole('list', { name: '缺失的作品' });
	await expect(works.getByRole('listitem')).toHaveCount(1);
	await works.getByRole('checkbox').check();
	await page.getByRole('button', { name: '清理所选（1）' }).click();
	const dialog = page.getByRole('dialog', { name: '清理缺失的条目？' });
	await expect(dialog).toContainText('收藏');
	await expect(dialog).toContainText('不能撤销');
	await shot(page, testInfo, 'missing');
	await dialog.getByRole('button', { name: '取消' }).click();
	await expect(dialog).toBeHidden();
	await expect(works.getByRole('listitem')).toHaveCount(1);

	await page.getByRole('button', { name: '清理所选（1）' }).click();
	await dialog.getByRole('button', { name: '永久删除' }).click();
	await expect(page.getByRole('status').filter({ hasText: '已删除 1 部作品、0 个位置。' })).toBeVisible();
	await expect(works).toHaveCount(0);
	await expect(page.getByRole('list', { name: '缺失的位置' }).getByRole('listitem')).toHaveCount(1);
	// The deleted work also leaves the favorites.
	await navigate(page, '收藏');
	await expect(page.getByRole('article')).toHaveCount(1);
});

test('cleanup waits for a running scan', async ({ page }) => {
	await openSettings(page);
	await page.getByRole('button', { name: '扫描 本地作品' }).click();
	await expect(card(page, '本地作品')).toContainText('进行中');
	await page.getByRole('list', { name: '缺失的位置' }).getByRole('checkbox').check();
	await page.getByRole('button', { name: '清理所选（1）' }).click();
	await page.getByRole('dialog').getByRole('button', { name: '永久删除' }).click();
	const blocked = page.getByRole('alert').filter({ hasText: '有扫描正在进行' });
	await expect(blocked).toBeVisible();
	await expect(page.getByRole('button', { name: '清理所选（1）' })).toBeVisible();
	await expect(card(page, '本地作品')).toContainText('已完成', { timeout: 15_000 });
	await blocked.getByRole('button', { name: '重试' }).click();
	await expect(page.getByRole('status').filter({ hasText: '已删除 0 部作品、1 个位置。' })).toBeVisible();
});

test('two tags are merged after confirming', async ({ page }, testInfo) => {
	await openSettings(page);
	await pick(page, '要合并的标签（合并后删除）', '耳语（8）', '耳语');
	await pick(page, '合并进', 'ASMR（120）', 'ASMR');
	await page.getByRole('button', { name: '合并', exact: true }).click();
	const dialog = page.getByRole('dialog', { name: '合并标签？' });
	await expect(dialog).toContainText('把「耳语」合并进「ASMR」');
	await shot(page, testInfo, 'tag-merge');
	await dialog.getByRole('button', { name: '合并', exact: true }).click();
	await expect(page.getByRole('status').filter({ hasText: '已把「耳语」合并进「ASMR」。' })).toBeVisible();
	await expect(pickerButton(page, '要合并的标签（合并后删除）')).toContainText('选择标签');
	await pickerButton(page, '要合并的标签（合并后删除）').click();
	const sheet = pickerSheet(page, '要合并的标签（合并后删除）');
	await sheet.getByRole('combobox').fill('耳语');
	await expect(sheet.getByRole('option')).toHaveCount(0);
	await sheet.getByRole('combobox').fill('ASMR');
	await expect(sheet.getByRole('option', { name: 'ASMR（128）', exact: true })).toBeVisible();
});

test('the tag picker searches, picks with the keyboard and lists other spellings first', async ({ page }, testInfo) => {
	await openSettings(page);
	await pickerButton(page, '要合并的标签（合并后删除）').click();
	const from = pickerSheet(page, '要合并的标签（合并后删除）');
	const search = from.getByRole('combobox');
	await expect(search).toBeFocused();
	await search.fill('耳');
	// Most used first, and the first match is ready for Enter.
	await expect(from.getByRole('option')).toHaveText([/^\s*耳语\s*8\s*$/, /^\s*耳語\s*3\s*$/]);
	// An Enter that confirms an IME word keeps the sheet open; the next one picks.
	await search.dispatchEvent('keydown', { key: 'Enter', isComposing: true, bubbles: true });
	await expect(from).toBeVisible();
	await expect(pickerButton(page, '要合并的标签（合并后删除）')).toContainText('选择标签');
	await search.press('Enter');
	await expect(from).toBeHidden();
	await expect(pickerButton(page, '要合并的标签（合并后删除）')).toContainText('耳语（8）');

	await pickerButton(page, '合并进').click();
	const into = pickerSheet(page, '合并进');
	const similar = into.getByRole('group', { name: '名字相近' });
	await expect(similar.getByRole('option')).toHaveText([/^\s*耳語\s*3\s*$/]);
	// The tag being merged away is not offered as its own target.
	await expect(into.getByRole('option', { name: '耳语（8）', exact: true })).toHaveCount(0);
	await shot(page, testInfo, 'tag-picker');
	await similar.getByRole('option', { name: '耳語（3）', exact: true }).click();
	await expect(pickerButton(page, '合并进')).toContainText('耳語（3）');
	await expect(page.getByText('作品和别名都会转到目标标签。')).toBeVisible();
});

test('metadata edits round-trip, and only a forced refresh replaces them', async ({ page }, testInfo) => {
	await signIn(page);
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await expect(page.getByRole('heading', { name: '示例作品一', level: 1 })).toBeVisible();
	await page.getByRole('button', { name: '编辑元数据' }).click();
	const dialog = page.getByRole('dialog', { name: '编辑元数据' });
	await expect(dialog.getByLabel('标题')).toHaveValue('示例作品一');
	await expect(dialog.getByLabel('声优')).toHaveValue('示例声优甲\n示例声优乙');

	await dialog.getByLabel('标题').fill('   ');
	await dialog.getByRole('button', { name: '保存' }).click();
	await expect(dialog.getByText('标题不能为空')).toBeVisible();

	await dialog.getByLabel('标题').fill('示例作品一（修正）');
	await dialog.getByLabel('声优').fill('示例声优甲\n新的声优');
	await dialog.getByLabel('发售日').fill('2024-06-01');
	await shot(page, testInfo, 'metadata-editor');
	await dialog.getByRole('button', { name: '保存' }).click();
	await expect(dialog).toBeHidden();
	await expect(page.getByRole('heading', { name: '示例作品一（修正）', level: 1 })).toBeVisible();
	await expect(page.getByRole('status').filter({ hasText: '普通的重新抓取不会覆盖它' })).toBeVisible();
	await expect(page.getByText(/新的声优/)).toBeVisible();

	// A fresh GET shows the saved data.
	await navigate(page, '资料库');
	await page.getByRole('link', { name: /示例作品一（修正）/ }).click();
	await expect(page.getByRole('heading', { name: '示例作品一（修正）', level: 1 })).toBeVisible();
	await expect(page.getByText('RJ01234567 · 2024-06-01')).toBeVisible();

	// A plain refresh keeps the manual edit.
	await page.getByRole('button', { name: '重新抓取' }).click();
	await expect(page.getByRole('status').filter({ hasText: '重新抓取：已完成' })).toBeVisible();
	await expect(page.getByRole('heading', { name: '示例作品一（修正）', level: 1 })).toBeVisible();

	// A forced one warns first, then brings back the fetched data.
	await page.getByLabel('强制覆盖手动修改').check();
	await expect(page.getByRole('note')).toContainText('覆盖手动修改过的元数据');
	await page.getByRole('button', { name: '重新抓取' }).click();
	const confirm = page.getByRole('dialog', { name: '强制重新抓取？' });
	await expect(confirm).toContainText('不能恢复');
	await confirm.getByRole('button', { name: '覆盖并抓取' }).click();
	await expect(page.getByRole('heading', { name: '示例作品一', level: 1 })).toBeVisible();
	await expect(page.getByText('示例声优甲、示例声优乙')).toBeVisible();
});

test('a plain refresh after a save also drops the notice, since the task may have been joined', async ({ page }) => {
	await signIn(page);
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await page.getByRole('button', { name: '编辑元数据' }).click();
	const dialog = page.getByRole('dialog', { name: '编辑元数据' });
	await dialog.getByLabel('标题').fill('示例作品一（修正）');
	await dialog.getByRole('button', { name: '保存' }).click();
	const notice = page.getByRole('status').filter({ hasText: '普通的重新抓取不会覆盖它' });
	await expect(notice).toBeVisible();
	await page.getByRole('button', { name: '重新抓取' }).click();
	await expect(page.getByRole('status').filter({ hasText: '重新抓取：已完成' })).toBeVisible();
	await expect(notice).toHaveCount(0);
	await expect(page.getByRole('heading', { name: '示例作品一（修正）', level: 1 })).toBeVisible();
});

test('a forced refresh right after a save removes the protection notice', async ({ page }) => {
	await signIn(page);
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await page.getByRole('button', { name: '编辑元数据' }).click();
	const dialog = page.getByRole('dialog', { name: '编辑元数据' });
	await dialog.getByLabel('标题').fill('示例作品一（修正）');
	await dialog.getByRole('button', { name: '保存' }).click();
	const notice = page.getByRole('status').filter({ hasText: '普通的重新抓取不会覆盖它' });
	await expect(notice).toBeVisible();
	await page.getByLabel('强制覆盖手动修改').check();
	await page.getByRole('button', { name: '重新抓取' }).click();
	await page.getByRole('dialog', { name: '强制重新抓取？' }).getByRole('button', { name: '覆盖并抓取' }).click();
	await expect(page.getByRole('heading', { name: '示例作品一', level: 1 })).toBeVisible();
	await expect(notice).toHaveCount(0);
});

test('a save that answers after moving to another work does not replace it', async ({ page }) => {
	await signIn(page);
	// Move from work 13 to work 12 by a client-side link, so going back later
	// stays on the detail route and the page component is reused.
	await page.getByRole('link', { name: /示例作品二/ }).click();
	await expect(page.getByRole('heading', { name: '示例作品二', level: 1 })).toBeVisible();
	await page.evaluate(() => {
		const link = Object.assign(document.createElement('a'), { href: '/works/12', textContent: 'to 12' });
		document.querySelector('main')?.append(link);
	});
	await page.getByRole('link', { name: 'to 12' }).click();
	await expect(page.getByRole('heading', { name: '示例作品一', level: 1 })).toBeVisible();
	await page.getByRole('button', { name: '编辑元数据' }).click();
	const dialog = page.getByRole('dialog', { name: '编辑元数据' });
	await dialog.getByLabel('标题').fill('示例作品一（修正）');
	await page.evaluate(() => ((globalThis as { __kikoeruMockLatencyMs?: number }).__kikoeruMockLatencyMs = 1_500));
	await dialog.getByRole('button', { name: '保存' }).click();
	await dialog.getByRole('button', { name: '取消' }).click();
	await page.evaluate(() => ((globalThis as { __kikoeruMockLatencyMs?: number }).__kikoeruMockLatencyMs = 0));
	await page.evaluate(() => history.back());
	await expect(page.getByRole('heading', { name: '示例作品二', level: 1 })).toBeVisible();
	await page.waitForTimeout(2_000);
	await expect(page).toHaveURL(/\/works\/13$/);
	await expect(page.getByRole('heading', { name: '示例作品二', level: 1 })).toBeVisible();
});

test('a slow save from an earlier opening leaves the reopened dialog alone', async ({ page }) => {
	await signIn(page);
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await page.getByRole('button', { name: '编辑元数据' }).click();
	const dialog = page.getByRole('dialog', { name: '编辑元数据' });
	await dialog.getByLabel('标题').fill('第一次修改');
	await page.evaluate(() => ((globalThis as { __kikoeruMockLatencyMs?: number }).__kikoeruMockLatencyMs = 1_500));
	await dialog.getByRole('button', { name: '保存' }).click();
	await dialog.getByRole('button', { name: '取消' }).click();
	await page.getByRole('button', { name: '编辑元数据' }).click();
	await dialog.getByLabel('声优').fill('正在写的草稿');
	await page.waitForTimeout(2_000);
	await expect(dialog).toBeVisible();
	await expect(dialog.getByLabel('声优')).toHaveValue('正在写的草稿');
	await expect(dialog.getByRole('button', { name: '保存' })).toBeEnabled();
	// The first save still reached the page.
	await expect(page.getByRole('heading', { name: '第一次修改', level: 1 })).toBeVisible();
});

test('the editor cannot reopen while a save is in flight', async ({ page }) => {
	await signIn(page);
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await page.getByRole('button', { name: '编辑元数据' }).click();
	const dialog = page.getByRole('dialog', { name: '编辑元数据' });
	await dialog.getByLabel('标题').fill('旧的修改');
	await page.evaluate(() => ((globalThis as { __kikoeruMockLatencyMs?: number }).__kikoeruMockLatencyMs = 1_500));
	await dialog.getByRole('button', { name: '保存' }).click();
	await dialog.getByRole('button', { name: '取消' }).click();
	await page.evaluate(() => ((globalThis as { __kikoeruMockLatencyMs?: number }).__kikoeruMockLatencyMs = 0));
	await expect(page.getByRole('button', { name: '编辑元数据' })).toBeDisabled();
	await expect(page.getByRole('heading', { name: '旧的修改', level: 1 })).toBeVisible({ timeout: 5_000 });
	await expect(page.getByRole('button', { name: '编辑元数据' })).toBeEnabled();
	await page.getByRole('button', { name: '编辑元数据' }).click();
	await expect(dialog.getByLabel('标题')).toHaveValue('旧的修改');
});

test('editing waits for a detail read that is still in flight', async ({ page }) => {
	await signIn(page);
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await expect(page.getByRole('heading', { name: '示例作品一', level: 1 })).toBeVisible();
	await page.evaluate(() => {
		(globalThis as { __kikoeruMockHoldMs?: Record<string, number> }).__kikoeruMockHoldMs = { 'GET /works/12': 1_500 };
	});
	// Marking it finished reloads the detail, and that read is held back.
	await page.getByRole('button', { name: '标记为听过' }).click();
	await expect(page.getByRole('button', { name: '编辑元数据' })).toBeDisabled();
	await expect(page.getByRole('button', { name: '编辑元数据' })).toBeEnabled({ timeout: 5_000 });
});

test('a late save from the previous work does not close the next work’s editor', async ({ page }) => {
	await signIn(page);
	await page.getByRole('link', { name: /示例作品二/ }).click();
	await expect(page.getByRole('heading', { name: '示例作品二', level: 1 })).toBeVisible();
	await page.evaluate(() => {
		const link = Object.assign(document.createElement('a'), { href: '/works/12', textContent: 'to 12' });
		document.querySelector('main')?.append(link);
	});
	await page.getByRole('link', { name: 'to 12' }).click();
	await page.getByRole('button', { name: '编辑元数据' }).click();
	const dialog = page.getByRole('dialog', { name: '编辑元数据' });
	await dialog.getByLabel('标题').fill('示例作品一（修正）');
	await page.evaluate(() => ((globalThis as { __kikoeruMockLatencyMs?: number }).__kikoeruMockLatencyMs = 1_500));
	await dialog.getByRole('button', { name: '保存' }).click();
	await dialog.getByRole('button', { name: '取消' }).click();
	await page.evaluate(() => ((globalThis as { __kikoeruMockLatencyMs?: number }).__kikoeruMockLatencyMs = 0));
	await page.evaluate(() => history.back());
	await expect(page.getByRole('heading', { name: '示例作品二', level: 1 })).toBeVisible();
	await page.getByRole('button', { name: '编辑元数据' }).click();
	await expect(dialog.getByLabel('标题')).toHaveValue('示例作品二');
	await page.waitForTimeout(2_000);
	await expect(dialog).toBeVisible();
});

/**
 * Keeps the mock from answering requests to `key` until the returned
 * function runs (`__kikoeruMockGates` in src/lib/api/mock.ts).
 */
async function holdRequest(page: Page, key: string): Promise<() => Promise<void>> {
	await page.evaluate((k) => {
		const hooks = globalThis as {
			__kikoeruMockGates?: Record<string, { promise: Promise<void>; release: () => void }>;
		};
		let release = () => {};
		const promise = new Promise<void>((resolve) => (release = resolve));
		hooks.__kikoeruMockGates = { ...hooks.__kikoeruMockGates, [k]: { promise, release } };
	}, key);
	return () =>
		page.evaluate((k) => {
			const hooks = globalThis as { __kikoeruMockGates?: Record<string, { release: () => void }> };
			hooks.__kikoeruMockGates?.[k]?.release();
		}, key);
}

test('leaving and coming back cannot start a second save of the same work', async ({ page }) => {
	await signIn(page);
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await page.getByRole('button', { name: '编辑元数据' }).click();
	const dialog = page.getByRole('dialog', { name: '编辑元数据' });
	await dialog.getByLabel('标题').fill('旧的修改');
	// Held for the rest of the test. A save on a timer could land while a
	// slow machine is still navigating and change the title checked below.
	await holdRequest(page, 'PUT /works/12/metadata');
	await dialog.getByRole('button', { name: '保存' }).click();
	await dialog.getByRole('button', { name: '取消' }).click();
	await page.getByRole('button', { name: '返回' }).click();
	await page.getByRole('link', { name: /示例作品一/ }).first().click();
	await expect(page.getByRole('heading', { name: '示例作品一', level: 1 })).toBeVisible();
	await expect(page.getByRole('button', { name: '编辑元数据' })).toBeDisabled();
});

test('a save sent from an earlier visit updates the page when it lands', async ({ page }) => {
	await signIn(page);
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await page.getByRole('button', { name: '编辑元数据' }).click();
	const dialog = page.getByRole('dialog', { name: '编辑元数据' });
	await dialog.getByLabel('标题').fill('示例作品一（修正）');
	const release = await holdRequest(page, 'PUT /works/12/metadata');
	await dialog.getByRole('button', { name: '保存' }).click();
	await dialog.getByRole('button', { name: '取消' }).click();
	await page.getByRole('button', { name: '返回' }).click();
	await page.getByRole('link', { name: /示例作品一/ }).first().click();
	await expect(page.getByRole('heading', { name: '示例作品一', level: 1 })).toBeVisible();
	// The save lands only after the revisited page has shown the old title.
	await release();
	await expect(page.getByRole('heading', { name: '示例作品一（修正）', level: 1 })).toBeVisible();
});

test('a late save answer does not undo progress shown after it', async ({ page }) => {
	await signIn(page);
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await page.getByRole('button', { name: '编辑元数据' }).click();
	const dialog = page.getByRole('dialog', { name: '编辑元数据' });
	await dialog.getByLabel('标题').fill('示例作品一（修正）');
	await page.evaluate(() => {
		(globalThis as { __kikoeruMockHoldMs?: Record<string, number> }).__kikoeruMockHoldMs = { 'PUT /works/12/metadata': 2_000 };
	});
	await dialog.getByRole('button', { name: '保存' }).click();
	await dialog.getByRole('button', { name: '取消' }).click();
	await page.getByRole('button', { name: '标记为听过' }).click();
	await expect(page.getByRole('button', { name: '已听过' })).toBeDisabled();
	await expect(page.getByRole('heading', { name: '示例作品一（修正）', level: 1 })).toBeVisible({ timeout: 5_000 });
	// Past the held PUT answer, which carries the progress from before.
	await page.waitForTimeout(2_500);
	await expect(page.getByRole('button', { name: '已听过' })).toBeDisabled();
});

test('the metadata fields are locked while a save is out', async ({ page }) => {
	await signIn(page);
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await page.getByRole('button', { name: '编辑元数据' }).click();
	const dialog = page.getByRole('dialog', { name: '编辑元数据' });
	await dialog.getByLabel('标题').fill('示例作品一（修正）');
	await page.evaluate(() => {
		(globalThis as { __kikoeruMockHoldMs?: Record<string, number> }).__kikoeruMockHoldMs = { 'PUT /works/12/metadata': 2_000 };
	});
	await dialog.getByRole('button', { name: '保存' }).click();
	// Anything typed now would be lost when the save closes the dialog.
	for (const label of ['标题', '社团', '声优', '标签', '发售日', 'R18 作品']) {
		await expect(dialog.getByLabel(label)).toBeDisabled();
	}
	await expect(dialog.getByRole('button', { name: '取消' })).toBeEnabled();
	await expect(dialog).toBeHidden({ timeout: 5_000 });
	await expect(page.getByRole('heading', { name: '示例作品一（修正）', level: 1 })).toBeVisible();
});

test('the tag choices reload when a scan finishes', async ({ page }) => {
	await openSettings(page);
	const from = pickerSheet(page, '要合并的标签（合并后删除）');
	await pickerButton(page, '要合并的标签（合并后删除）').click();
	await from.getByRole('combobox').fill('扫描发现');
	await expect(from.getByText('没有匹配的标签')).toBeVisible();
	await from.getByRole('button', { name: '取消' }).click();
	await page.getByRole('button', { name: '扫描 本地作品' }).click();
	await expect(card(page, '本地作品')).toContainText('已完成', { timeout: 15_000 });
	// The mock's finished scan adds this tag (MOCK_SCANNED_TAG in mock.ts).
	await pickerButton(page, '要合并的标签（合并后删除）').click();
	await from.getByRole('combobox').fill('扫描发现');
	await expect(from.getByRole('option', { name: '扫描发现的标签（1）', exact: true })).toHaveCount(1);
});

test('a refresh waits for a metadata save that is still in flight', async ({ page }) => {
	await signIn(page);
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await page.getByRole('button', { name: '编辑元数据' }).click();
	const dialog = page.getByRole('dialog', { name: '编辑元数据' });
	await dialog.getByLabel('标题').fill('示例作品一（修正）');
	await page.evaluate(() => ((globalThis as { __kikoeruMockLatencyMs?: number }).__kikoeruMockLatencyMs = 1_500));
	await dialog.getByRole('button', { name: '保存' }).click();
	await dialog.getByRole('button', { name: '取消' }).click();
	await expect(page.getByRole('button', { name: '重新抓取' })).toBeDisabled();
	await expect(page.getByText('保存完成后才能重新抓取')).toBeVisible();
	await page.evaluate(() => ((globalThis as { __kikoeruMockLatencyMs?: number }).__kikoeruMockLatencyMs = 0));
	await expect(page.getByRole('button', { name: '重新抓取' })).toBeEnabled({ timeout: 5_000 });
});

test('editing waits for a running refresh', async ({ page }) => {
	await signIn(page);
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await page.getByRole('button', { name: '重新抓取' }).click();
	await expect(page.getByRole('button', { name: '编辑元数据' })).toBeDisabled();
	await expect(page.getByRole('status').filter({ hasText: '重新抓取：已完成' })).toBeVisible();
	await expect(page.getByRole('button', { name: '编辑元数据' })).toBeEnabled();
});

test('a refresh started before leaving the page still locks the editor on return', async ({ page }) => {
	await signIn(page);
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await page.getByRole('button', { name: '重新抓取' }).click();
	await expect(page.getByRole('button', { name: '编辑元数据' })).toBeDisabled();
	await page.getByRole('button', { name: '返回' }).click();
	await page.getByRole('link', { name: /示例作品一/ }).first().click();
	await expect(page.getByRole('heading', { name: '示例作品一', level: 1 })).toBeVisible();
	await expect(page.getByRole('button', { name: '编辑元数据' })).toBeDisabled();
	await expect(page.getByRole('status').filter({ hasText: '重新抓取：已完成' })).toBeVisible();
	await expect(page.getByRole('button', { name: '编辑元数据' })).toBeEnabled();
});

test('a refresh whose answer arrives after returning to the page is still followed', async ({ page }) => {
	await signIn(page);
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await page.evaluate(() => {
		(globalThis as { __kikoeruMockHoldMs?: Record<string, number> }).__kikoeruMockHoldMs = {
			'POST /works/12/refresh': 2_000
		};
	});
	await page.getByRole('button', { name: '重新抓取' }).click();
	await page.getByRole('button', { name: '返回' }).click();
	await page.getByRole('link', { name: /示例作品一/ }).first().click();
	await expect(page.getByRole('heading', { name: '示例作品一', level: 1 })).toBeVisible();
	// The first request is still out, so a second refresh cannot start.
	await expect(page.getByRole('button', { name: '正在抓取' })).toBeDisabled();
	await expect(page.getByRole('status').filter({ hasText: '重新抓取：已完成' })).toBeVisible({ timeout: 8_000 });
	await expect(page.getByRole('button', { name: '编辑元数据' })).toBeEnabled();
});

test('a refresh whose task cannot be followed can be tried again', async ({ page }) => {
	await signIn(page);
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await page.evaluate(() => {
		(globalThis as { __kikoeruMockStreamGoneOnce?: string }).__kikoeruMockStreamGoneOnce = 'GET /tasks/';
	});
	await page.getByRole('button', { name: '重新抓取' }).click();
	await expect(page.getByRole('alert').filter({ hasText: '无法读取抓取进度' })).toBeVisible();
	await expect(page.getByRole('button', { name: '重新抓取' })).toBeEnabled();
	await expect(page.getByRole('button', { name: '编辑元数据' })).toBeEnabled();
});

test('the editor reopens only after the saved metadata is read back', async ({ page }) => {
	await signIn(page);
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await page.getByRole('button', { name: '编辑元数据' }).click();
	const dialog = page.getByRole('dialog', { name: '编辑元数据' });
	await dialog.getByLabel('标题').fill('示例作品一（修正）');
	await page.evaluate(() => {
		(globalThis as { __kikoeruMockHoldMs?: Record<string, number> }).__kikoeruMockHoldMs = { 'GET /works/12': 1_500 };
	});
	await dialog.getByRole('button', { name: '保存' }).click();
	await expect(dialog).toBeHidden();
	await expect(page.getByRole('button', { name: '编辑元数据' })).toBeDisabled();
	await expect(page.getByRole('button', { name: '编辑元数据' })).toBeEnabled({ timeout: 5_000 });
	await page.getByRole('button', { name: '编辑元数据' }).click();
	await expect(dialog.getByLabel('标题')).toHaveValue('示例作品一（修正）');
});

test('a listener sees no admin controls', async ({ page }) => {
	await signIn(page, 'listener');
	await navigate(page, '设置');
	await expect(page.getByRole('button', { name: '退出登录' })).toBeVisible();
	await expect(page.getByRole('heading', { name: '媒体库' })).toHaveCount(0);
	await expect(page.getByRole('heading', { name: '任务' })).toHaveCount(0);
	await navigate(page, '资料库');
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await expect(page.getByRole('heading', { name: '示例作品一', level: 1 })).toBeVisible();
	await expect(page.getByRole('button', { name: '编辑元数据' })).toHaveCount(0);
	await expect(page.getByRole('button', { name: '重新抓取' })).toHaveCount(0);
});
