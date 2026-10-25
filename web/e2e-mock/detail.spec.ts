import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Locator, type Page, type TestInfo } from '@playwright/test';

// The work detail page's layout and its progress actions. 示例作品一 (work 12)
// has saved progress, tags and a review in the samples.

async function signIn(page: Page, name = 'admin') {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill(name);
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

async function openWork(page: Page) {
	await page.goto('/works/12');
	await expect(page.getByRole('heading', { name: '曲目' })).toBeVisible();
}

async function shot(page: Page, testInfo: TestInfo, name: string) {
	const dir = process.env.KIKOERU_MOCK_SCREENSHOTS;
	if (!dir) return;
	mkdirSync(dir, { recursive: true });
	await page.screenshot({ path: join(dir, `detail-${name}-${testInfo.project.name}.png`) });
}

async function box(locator: Locator) {
	const b = await locator.boundingBox();
	if (!b) throw new Error('not rendered');
	return b;
}

const cover = (page: Page) => page.getByRole('article').locator(':scope > *').first();
const title = (page: Page) => page.getByRole('heading', { level: 1 });
const region = (page: Page, name: string) => page.getByRole('region', { name, exact: true });
// The mock answers inside the page, so its own counter stands in for the network log.
const deletes = (page: Page) =>
	page.evaluate(
		() =>
			(globalThis as { __kikoeruMockRequests?: Record<string, number> }).__kikoeruMockRequests?.[
				'DELETE /me/progress/12'
			] ?? 0
	);

test('wide windows line the groups up with the cover and the info columns', async ({ page }, testInfo) => {
	test.skip(testInfo.project.name !== 'desktop', 'the two columns only exist on wide windows');
	await signIn(page);
	await openWork(page);
	const coverBox = await box(cover(page));
	const titleBox = await box(title(page));
	expect(titleBox.x).toBeGreaterThan(coverBox.x + coverBox.width);

	// 我的评价 sits under the cover at its width, 管理 under the info, and
	// both start on the same line.
	const review = await box(region(page, '我的评价'));
	const admin = await box(region(page, '管理'));
	expect(review.x).toBeCloseTo(coverBox.x, 0);
	expect(review.width).toBeCloseTo(coverBox.width, 0);
	expect(admin.x).toBeCloseTo(titleBox.x, 0);
	expect(admin.y).toBeCloseTo(review.y, 0);

	// The track list spans both columns.
	const tracks = await box(region(page, '曲目'));
	expect(tracks.x).toBeCloseTo(coverBox.x, 0);
	expect(tracks.x + tracks.width).toBeCloseTo(admin.x + admin.width, 0);
	await shot(page, testInfo, 'layout');
});

test('a window with the sidebar but too narrow for two columns stacks like a phone', async ({ page }, testInfo) => {
	test.skip(testInfo.project.name !== 'desktop', 'the sidebar only shows on desktop');
	await page.setViewportSize({ width: 820, height: 1180 });
	await signIn(page);
	await openWork(page);
	await expect(page.getByRole('navigation', { name: '主导航' })).toBeVisible();
	const coverBox = await box(cover(page));
	const titleBox = await box(title(page));
	// The info used to be squeezed into a column a few characters wide here.
	expect(titleBox.y).toBeGreaterThan(coverBox.y + coverBox.height);
	expect(titleBox.width).toBeGreaterThanOrEqual(coverBox.width - 1);
	const review = await box(region(page, '我的评价'));
	expect(review.width).toBeCloseTo(coverBox.width, 0);
});

test('clearing progress asks first in the app dialog, and cancel keeps it', async ({ page }, testInfo) => {
	await signIn(page);
	await openWork(page);
	await expect(page.getByRole('button', { name: '继续播放' })).toBeVisible();

	await page.getByRole('button', { name: '清除播放进度' }).click();
	const dialog = page.getByRole('dialog', { name: '清除播放进度？' });
	await expect(dialog).toBeVisible();
	await shot(page, testInfo, 'clear-confirm');
	await dialog.getByRole('button', { name: '取消' }).click();
	await expect(dialog).toBeHidden();
	expect(await deletes(page)).toBe(0);
	await expect(page.getByRole('button', { name: '继续播放' })).toBeVisible();

	await page.getByRole('button', { name: '清除播放进度' }).click();
	await dialog.getByRole('button', { name: '清除', exact: true }).click();
	// Without progress the page offers a fresh start and no clear button.
	await expect(page.getByRole('button', { name: '播放', exact: true })).toBeVisible();
	await expect(page.getByRole('button', { name: '清除播放进度' })).toHaveCount(0);
	expect(await deletes(page)).toBe(1);
});

test('admins see where the work is', async ({ page }, testInfo) => {
	await signIn(page);
	await openWork(page);
	// The sample has a present location and a missing one, present first.
	const row = page.getByRole('definition').filter({ hasText: '示例社团/RJ01234567' });
	await expect(row.getByRole('listitem')).toHaveText(['本地作品 · 示例社团/RJ01234567', '云端作品 · RJ01234567（已缺失）']);
	await expect(page.getByText('位置', { exact: true })).toBeVisible();
	await shot(page, testInfo, 'locations');

	// A folder name with no break opportunity must not widen the page.
	await page.setViewportSize({ width: 375, height: 800 });
	await row
		.getByRole('listitem')
		.first()
		.evaluate((item) => (item.textContent = `本地作品 · ${'x'.repeat(120)}`));
	expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(375);
});

test('a listener sees no locations', async ({ page }) => {
	await signIn(page, 'listener');
	await openWork(page);
	await expect(page.getByText('时长', { exact: true })).toBeVisible();
	await expect(page.getByText('位置', { exact: true })).toHaveCount(0);
});
