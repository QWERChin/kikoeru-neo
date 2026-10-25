import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page, type TestInfo } from '@playwright/test';

// Sample titles come from contract/; "合成作品" are the mock's synthetic
// works (src/lib/api/mock-catalog.ts): 90 in all, 56 of them by 示例社团.

async function signIn(page: Page) {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill('admin');
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

// The mock keeps favorites in memory, so tests move through the app's own
// navigation instead of page.goto, which would reload it.
async function navigate(page: Page, label: string) {
	await page.getByRole('navigation', { name: '主导航' }).getByRole('link', { name: label, exact: true }).click();
	await expect(page.getByRole('heading', { name: label === '最近播放' ? '最近' : label, level: 1 })).toBeVisible();
}

// A search filter is a ListPicker: a button that opens a sheet of choices.
const filterButton = (page: Page, label: string) => page.getByRole('button', { name: new RegExp(`^${label} `) });

async function pickFilter(page: Page, label: string, option: string) {
	await filterButton(page, label).click();
	const sheet = page.getByRole('dialog', { name: label, exact: true });
	await sheet.getByRole('option', { name: option, exact: true }).click();
	await expect(sheet).toBeHidden();
}

async function shot(page: Page, testInfo: TestInfo, name: string) {
	const dir = process.env.KIKOERU_MOCK_SCREENSHOTS;
	if (!dir) return;
	mkdirSync(dir, { recursive: true });
	await page.waitForLoadState('networkidle');
	await page.screenshot({ path: join(dir, `m3-${name}-${testInfo.project.name}.png`) });
}

const card = (page: Page, title: string) =>
	page.getByRole('article').filter({ has: page.getByRole('heading', { name: title, exact: true }) });

/** Waits until the grid shows the answer for the current URL, not the previous one. */
async function settled(page: Page) {
	await expect(page.locator('ul[aria-busy="false"]').filter({ has: page.getByRole('article') })).toBeVisible();
}

const searchBox = (page: Page) => page.getByRole('searchbox', { name: '搜索作品' });

test('favorites list the samples and follow a toggle on a card and the detail page', async ({ page }, testInfo) => {
	await signIn(page);
	await navigate(page, '收藏');
	await expect(page.getByRole('article')).toHaveCount(2);
	await expect(page.getByRole('article').first()).toContainText('示例作品一');
	// The missing work stays in the favorites, marked as missing.
	await expect(page.getByRole('article').nth(1)).toContainText('已缺失');
	await shot(page, testInfo, 'favorites');

	await navigate(page, '资料库');
	const heart = card(page, '示例作品二').getByRole('button', { name: '收藏' });
	await expect(heart).toHaveAttribute('aria-pressed', 'false');
	await heart.click();
	await expect(heart).toHaveAttribute('aria-pressed', 'true');
	// The button is disabled until the server answers.
	await expect(heart).toBeEnabled();
	await expect(card(page, '示例作品二').getByRole('img', { name: /已收藏/ })).toBeVisible();

	await navigate(page, '收藏');
	await expect(page.getByRole('article')).toHaveCount(3);
	// Newest addition first.
	await expect(page.getByRole('article').first()).toContainText('示例作品二');

	await card(page, '示例作品二').getByRole('link').click();
	await expect(page).toHaveURL(/\/works\/13$/);
	const detailHeart = page.getByRole('button', { name: '收藏' });
	await expect(detailHeart).toHaveAttribute('aria-pressed', 'true');
	await detailHeart.click();
	await expect(detailHeart).toHaveAttribute('aria-pressed', 'false');
	await expect(detailHeart).toBeEnabled();

	await navigate(page, '收藏');
	await expect(page.getByRole('article')).toHaveCount(2);
	await expect(card(page, '示例作品二')).toHaveCount(0);
});

test('search combines text, a filter, a sort and paging, and back restores each step', async ({ page }, testInfo) => {
	await signIn(page);
	if (testInfo.project.name === 'phone') await page.getByRole('link', { name: '搜索', exact: true }).click();
	await searchBox(page).fill('合成');
	await expect(page).toHaveURL(/\/search\?q=%E5%90%88%E6%88%90$/);
	await expect(page.getByText('第 1 / 3 页 · 共 90 部')).toBeVisible();

	await pickFilter(page, '社团', '示例社团（12）');
	await expect(page).toHaveURL(/[?&]circle=3(&|$)/);
	await expect(page.getByText('第 1 / 2 页 · 共 56 部')).toBeVisible();

	await page.getByLabel('排序').selectOption({ label: '发售日' });
	await expect(page).toHaveURL(/[?&]sort=release(&|$)/);
	await settled(page);
	await shot(page, testInfo, 'search');
	const firstOnPage1 = await page.getByRole('article').first().getByRole('heading').textContent();

	await page.getByRole('link', { name: '下一页' }).click();
	await expect(page).toHaveURL(/[?&]page=2(&|$)/);
	await expect(page.getByText('第 2 / 2 页 · 共 56 部')).toBeVisible();
	await expect(page.getByRole('article')).toHaveCount(16);

	// Refresh keeps every part of the search.
	await page.reload();
	await expect(page.getByText('第 2 / 2 页 · 共 56 部')).toBeVisible();
	await expect(searchBox(page)).toHaveValue('合成');

	await page.goBack();
	await expect(page).not.toHaveURL(/page=2/);
	await expect(page).toHaveURL(/sort=release/);
	await expect(page.getByRole('article').first().getByRole('heading')).toHaveText(firstOnPage1 ?? '');
	await expect(filterButton(page, '社团')).toContainText('示例社团（12）');

	await page.goBack();
	await expect(page).not.toHaveURL(/sort=/);
	await expect(page.getByLabel('排序')).toHaveValue('added');
	await page.goBack();
	await expect(page).not.toHaveURL(/circle=/);
	await expect(page.getByText('第 1 / 3 页 · 共 90 部')).toBeVisible();
	await expect(searchBox(page)).toHaveValue('合成');
});

test('a tag filter is found by searching and cleared with 全部', async ({ page }, testInfo) => {
	await signIn(page);
	if (testInfo.project.name === 'phone') await page.getByRole('link', { name: '搜索', exact: true }).click();
	await searchBox(page).fill('合成');
	await expect(page).toHaveURL(/q=%E5%90%88%E6%88%90$/);
	await filterButton(page, '标签').click();
	const sheet = page.getByRole('dialog', { name: '标签', exact: true });
	await sheet.getByRole('combobox').fill('合成标签 3');
	// 合成标签 30 to 39; the search ignores the space, so 合成标签 03 does not match.
	await expect(sheet.getByRole('option')).toHaveCount(10);
	await sheet.getByRole('option', { name: '合成标签 31（1）', exact: true }).click();
	await expect(page).toHaveURL(/[?&]tag=2030(&|$)/);
	await expect(page.getByText('第 1 / 1 页 · 共 1 部')).toBeVisible();
	await expect(filterButton(page, '标签')).toContainText('合成标签 31（1）');

	await pickFilter(page, '标签', '全部');
	await expect(page).not.toHaveURL(/tag=/);
	await expect(filterButton(page, '标签')).toContainText('全部');
});

test('back cancels text that was still waiting and shows the URL’s query again', async ({ page }, testInfo) => {
	await signIn(page);
	if (testInfo.project.name === 'phone') await page.getByRole('link', { name: '搜索', exact: true }).click();
	await searchBox(page).fill('合成');
	await expect(page).toHaveURL(/q=%E5%90%88%E6%88%90$/);
	await pickFilter(page, '标签', 'ASMR（120）');
	await expect(page).toHaveURL(/tag=7/);
	// Back lands on a URL with the same q, before the new text is sent.
	await searchBox(page).fill('其他');
	await page.goBack();
	await expect(page).not.toHaveURL(/tag=/);
	await expect(searchBox(page)).toHaveValue('合成');
	await page.waitForTimeout(600);
	await expect(page).toHaveURL(/q=%E5%90%88%E6%88%90$/);
});

test('a random order keeps its seed in the URL, so a page survives a reload', async ({ page }) => {
	await signIn(page);
	await page.getByLabel('排序').selectOption({ label: '随机' });
	await expect(page).toHaveURL(/\/library\?sort=random&seed=\d+$/);
	await page.getByRole('link', { name: '下一页' }).click();
	await expect(page).toHaveURL(/seed=\d+&page=2$/);
	await settled(page);
	const titles = await page.getByRole('article').getByRole('heading').allTextContents();
	await page.reload();
	await expect(page.getByRole('article').first().getByRole('heading')).toHaveText(titles[0] ?? '');
	expect(await page.getByRole('article').getByRole('heading').allTextContents()).toEqual(titles);
});

test('an unreachable media library shows a notice and keeps the session', async ({ page }, testInfo) => {
	await signIn(page);
	if (testInfo.project.name === 'phone') await page.getByRole('link', { name: '搜索', exact: true }).click();
	// The mock answers this search with 503 storage_unavailable.
	await searchBox(page).fill('503');
	const alert = page.getByRole('alert').filter({ hasText: '媒体库暂时无法访问' });
	await expect(alert).toContainText('媒体库 云端作品 暂时无法访问');
	await expect(alert).toContainText('作品、收藏和播放进度都还在');
	await expect(page.getByRole('heading', { name: '搜索', level: 1 })).toBeVisible();
	expect(await page.evaluate(() => localStorage.getItem('kikoeru-neo.token'))).not.toBeNull();

	// Works in that library cannot be prepared either, and the player says why.
	await searchBox(page).fill('示例作品二');
	await card(page, '示例作品二').getByRole('link').click();
	await page.getByRole('button', { name: '播放', exact: true }).click();
	await expect(page.getByRole('region', { name: '迷你播放条' })).toContainText('媒体库 云端作品 暂时无法访问');
});

test('recent lists played works and continues one', async ({ page }, testInfo) => {
	await signIn(page);
	await navigate(page, testInfo.project.name === 'phone' ? '最近' : '最近播放');
	await expect(page.getByRole('article').first()).toContainText('示例作品一');
	await expect(page.getByRole('article').first()).toContainText('13:32 / 33:15');
	await shot(page, testInfo, 'recent');
	await page.getByRole('button', { name: '继续播放《示例作品一》' }).click();
	const mini = page.getByRole('region', { name: '迷你播放条' });
	await expect(mini).toContainText('Tr02');
	await expect(page.getByRole('button', { name: '继续播放《示例作品一》' })).toContainText('正在播放');
});

test('settings shows the status of each media library to an admin', async ({ page }) => {
	await signIn(page);
	await navigate(page, '设置');
	const list = page.getByRole('list', { name: '媒体库状态' });
	await expect(list.getByRole('listitem')).toHaveCount(3);
	await expect(list.getByRole('listitem').nth(0)).toContainText('本地作品');
	await expect(list.getByRole('listitem').nth(0)).toContainText('正常');
	await expect(list.getByRole('listitem').nth(1)).toContainText('还没检查');
});

for (const viewport of [
	{ width: 1280, height: 720 },
	{ width: 1440, height: 900 }
]) {
	test(`the full player fits a ${viewport.width}x${viewport.height} window without scrolling`, async ({ page }, testInfo) => {
		test.skip(testInfo.project.name === 'phone', 'desktop layout only');
		await page.setViewportSize(viewport);
		await signIn(page);
		await page.getByRole('link', { name: /示例作品一/ }).click();
		await page.getByRole('button', { name: '继续播放' }).click();
		await page.getByRole('region', { name: '迷你播放条' }).getByRole('link', { name: /打开播放页/ }).click();
		await expect(page).toHaveURL(/\/player$/);
		await expect(page.getByRole('heading', { name: 'Tr02', level: 1 })).toBeVisible();
		await shot(page, testInfo, `player-${viewport.width}x${viewport.height}`);
		await expect(page.getByText('正在播放', { exact: true })).toBeInViewport();
		await expect(page.getByRole('button', { name: '下一轨' })).toBeInViewport();
		const fits = await page.evaluate(() => {
			const root = document.scrollingElement ?? document.documentElement;
			return { scroll: root.scrollHeight, inner: window.innerHeight };
		});
		expect(fits.scroll).toBeLessThanOrEqual(fits.inner);
		// The cover sits close to the title, and the option pills keep clear of the bottom edge.
		const cover = await page.locator('.cover-fit').boundingBox();
		const title = await page.getByRole('heading', { name: 'Tr02', level: 1 }).boundingBox();
		const speed = await page.getByRole('button', { name: /^倍速/ }).boundingBox();
		expect(cover && title && title.y - (cover.y + cover.height)).toBeLessThanOrEqual(48);
		expect(speed && viewport.height - (speed.y + speed.height)).toBeGreaterThanOrEqual(32);
	});
}

test('the selected player tab is lighter than the bar in dark mode', async ({ page }, testInfo) => {
	test.skip(testInfo.project.name === 'phone', 'one viewport is enough');
	await page.emulateMedia({ colorScheme: 'dark' });
	await signIn(page);
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await page.getByRole('button', { name: '继续播放' }).click();
	await page.getByRole('region', { name: '迷你播放条' }).getByRole('link', { name: /打开播放页/ }).click();
	const luminance = (selector: string) =>
		page.locator(selector).evaluate((el) => {
			const [r, g, b] = getComputedStyle(el).backgroundColor.match(/\d+/g)!.map(Number);
			return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!;
		});
	const bar = await luminance('[role="tablist"]');
	const selected = await luminance('[role="tab"][aria-selected="true"]');
	expect(selected).toBeGreaterThan(bar);
});
