import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page, type TestInfo } from '@playwright/test';

// The theme choice in 设置 > 外观. It lives in this browser's storage, so
// every test starts from 跟随系统.

async function signIn(page: Page) {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill('admin');
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

async function openSettings(page: Page) {
	await page.locator('a[href="/settings"]:visible').first().click();
	await expect(page.getByRole('heading', { name: '设置', level: 1 })).toBeVisible();
}

async function shot(page: Page, testInfo: TestInfo, name: string) {
	const dir = process.env.KIKOERU_MOCK_SCREENSHOTS;
	if (!dir) return;
	mkdirSync(dir, { recursive: true });
	await page.getByRole('region', { name: '外观' }).scrollIntoViewIfNeeded();
	await page.screenshot({ path: join(dir, `theme-${name}-${testInfo.project.name}.png`) });
}

const rootTheme = (page: Page) => page.evaluate(() => document.documentElement.dataset.theme ?? null);
const background = (page: Page) => page.evaluate(() => getComputedStyle(document.body).backgroundColor);
const barColor = (page: Page) =>
	page.evaluate(() => document.querySelector('meta[name="theme-color"]')?.getAttribute('content') ?? null);
const accentText = (page: Page) =>
	page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--accent-text').trim());
const themes = (page: Page) => page.getByRole('radiogroup', { name: '主题' });

test('跟随系统 follows system changes live', async ({ page }, testInfo) => {
	await page.emulateMedia({ colorScheme: 'light' });
	await signIn(page);
	await openSettings(page);
	await expect(themes(page).getByRole('radio', { name: '跟随系统' })).toHaveAttribute('aria-checked', 'true');
	expect(await rootTheme(page)).toBe('light');
	expect(await background(page)).toBe('rgb(242, 242, 247)');

	await page.emulateMedia({ colorScheme: 'dark' });
	await expect.poll(() => rootTheme(page)).toBe('dark');
	expect(await background(page)).toBe('rgb(0, 0, 0)');
	expect(await barColor(page)).toBe('#000000');
	await shot(page, testInfo, 'system-dark');

	await page.emulateMedia({ colorScheme: 'light' });
	await expect.poll(() => rootTheme(page)).toBe('light');
	expect(await barColor(page)).toBe('#f2f2f7');
});

test('a picked theme overrides the system and holds from the first paint after a reload', async ({ page }, testInfo) => {
	await page.emulateMedia({ colorScheme: 'light' });
	await signIn(page);
	await openSettings(page);
	await themes(page).getByRole('radio', { name: '深色' }).click();
	await expect(themes(page).getByRole('radio', { name: '深色' })).toHaveAttribute('aria-checked', 'true');
	await expect.poll(() => rootTheme(page)).toBe('dark');
	expect(await background(page)).toBe('rgb(0, 0, 0)');
	// The accent switches to its tint for dark backgrounds as well.
	expect(await accentText(page)).toBe('#5aa5f7');
	await shot(page, testInfo, 'dark');

	// A system change does not undo the choice.
	await page.emulateMedia({ colorScheme: 'dark' });
	await page.emulateMedia({ colorScheme: 'light' });
	expect(await rootTheme(page)).toBe('dark');

	// With every script but static/theme.js blocked, the page is already dark,
	// so the app's own start cannot be what applies it.
	await page.route('**/*', (route) => {
		const request = route.request();
		if (request.resourceType() === 'document' || new URL(request.url()).pathname === '/theme.js') {
			return route.continue();
		}
		return route.abort();
	});
	await page.reload();
	expect(await rootTheme(page)).toBe('dark');
	expect(await barColor(page)).toBe('#000000');
});

test('浅色 holds on a dark system', async ({ page }) => {
	await page.emulateMedia({ colorScheme: 'dark' });
	await signIn(page);
	expect(await rootTheme(page)).toBe('dark');
	await openSettings(page);
	await themes(page).getByRole('radio', { name: '浅色' }).click();
	await expect.poll(() => rootTheme(page)).toBe('light');
	expect(await background(page)).toBe('rgb(242, 242, 247)');
	expect(await accentText(page)).toBe('#0a63c9');
	await page.reload();
	await expect(page.getByRole('heading', { name: '设置', level: 1 })).toBeVisible();
	expect(await rootTheme(page)).toBe('light');
	await expect(themes(page).getByRole('radio', { name: '浅色' })).toHaveAttribute('aria-checked', 'true');
});

test('设置 opens with 外观, even above an admin\'s library groups on a phone', async ({ page }, testInfo) => {
	await signIn(page);
	await openSettings(page);
	// The sidebar has a 媒体库 group of its own.
	const main = page.getByRole('main');
	const appearance = await main.getByRole('region', { name: '外观', exact: true }).boundingBox();
	const libraries = await main.getByRole('region', { name: '媒体库', exact: true }).boundingBox();
	const playback = await main.getByRole('region', { name: '播放', exact: true }).boundingBox();
	if (!appearance || !libraries || !playback) throw new Error('not rendered');
	if (testInfo.project.name === 'phone') {
		expect(appearance.y).toBeLessThan(libraries.y);
	} else {
		// Two columns: 外观 heads the right one, beside the libraries.
		expect(appearance.x).toBeGreaterThan(libraries.x + libraries.width);
		expect(appearance.y).toBeLessThan(playback.y);
		expect(Math.abs(appearance.y - libraries.y)).toBeLessThan(40);
	}
});

test('a long library name still fits the settings page on a phone', async ({ page }, testInfo) => {
	test.skip(testInfo.project.name !== 'phone', 'the one-column layout is what lost its width limit');
	await signIn(page);
	await openSettings(page);
	// 70 characters without a break, under the 100 a name may have.
	const longName = 'VeryLongLibraryNameWithoutAnySpaces'.repeat(2);
	await page.getByRole('button', { name: '添加媒体库' }).click();
	const sheet = page.getByRole('dialog', { name: /媒体库/ });
	await sheet.getByLabel('名称', { exact: true }).fill(longName);
	await sheet.getByLabel('路径', { exact: true }).fill('/srv/media/voice');
	await sheet.getByRole('button', { name: '添加' }).click();
	await expect(sheet).toBeHidden();
	const row = page.getByRole('list', { name: '媒体库状态' }).getByRole('listitem').filter({ hasText: longName });
	await expect(row).toBeVisible();
	// The name truncates instead of widening the page past the screen.
	const viewport = page.viewportSize()!.width;
	expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(viewport);
	const scan = await row.getByRole('button', { name: /^扫描/ }).boundingBox();
	expect(scan!.x + scan!.width).toBeLessThanOrEqual(viewport);
});
