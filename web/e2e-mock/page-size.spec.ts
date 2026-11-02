import { expect, test, type Page } from '@playwright/test';

// The mock library has 93 works: the three contract samples and the 90
// synthetic works of src/lib/api/mock-catalog.ts.

async function signIn(page: Page) {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill('admin');
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

// The mock keeps the settings in memory, so the test moves through the
// app's own links instead of page.goto, which would reload it.
async function openSettings(page: Page) {
	await page.locator('a[href="/settings"]:visible').first().click();
	await expect(page.getByRole('heading', { name: '设置', level: 1 })).toBeVisible();
}

async function openLibrary(page: Page) {
	await page.locator('a[href="/library"]:visible').first().click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

const cards = (page: Page) => page.locator('ul[aria-busy="false"]').getByRole('article');
const worksRequests = (page: Page) =>
	page.evaluate(
		() => (globalThis as { __kikoeruMockRequests?: Record<string, number> }).__kikoeruMockRequests?.['GET /works'] ?? 0
	);

test('the chosen page size pages the library and is saved with the account', async ({ page }) => {
	await signIn(page);
	await expect(page.getByText('第 1 / 3 页 · 共 93 部')).toBeVisible();
	await expect(cards(page)).toHaveCount(40);

	await openSettings(page);
	const group = page.getByRole('radiogroup', { name: '每页作品数' });
	await expect(group.getByRole('radio', { name: '40' })).toHaveAttribute('aria-checked', 'true');
	await group.getByRole('radio', { name: '20' }).click();
	await expect(group.getByRole('radio', { name: '20' })).toHaveAttribute('aria-checked', 'true');

	const before = await worksRequests(page);
	await openLibrary(page);
	await expect(page.getByText('第 1 / 5 页 · 共 93 部')).toBeVisible();
	await expect(cards(page)).toHaveCount(20);
	// The settings were already known, so the list asked only once.
	expect(await worksRequests(page)).toBe(before + 1);

	await openSettings(page);
	await page.getByRole('radiogroup', { name: '每页作品数' }).getByRole('radio', { name: '100' }).click();
	await openLibrary(page);
	await expect(page.getByText('第 1 / 1 页 · 共 93 部')).toBeVisible();
	await expect(cards(page)).toHaveCount(93);
});

test('a list still loads at the default size when the settings cannot be read', async ({ page }) => {
	await page.addInitScript(() => {
		(globalThis as { __kikoeruMockFailOnce?: string }).__kikoeruMockFailOnce = 'GET /me/settings';
	});
	await signIn(page);
	await expect(page.getByText('第 1 / 3 页 · 共 93 部')).toBeVisible();
	await expect(cards(page)).toHaveCount(40);
});

test('a list waits for the account settings, then loads once', async ({ page }) => {
	// Held from the first page load, so the settings are still out after signing in.
	await page.addInitScript(() => {
		const hooks = globalThis as {
			__kikoeruMockGates?: Record<string, { promise: Promise<void>; release: () => void }>;
		};
		let release = () => {};
		const promise = new Promise<void>((resolve) => (release = resolve));
		hooks.__kikoeruMockGates = { 'GET /me/settings': { promise, release } };
	});
	await signIn(page);
	await expect(page.getByRole('list', { name: '正在加载' })).toBeVisible();
	expect(await worksRequests(page)).toBe(0);

	await page.evaluate(() => {
		const hooks = globalThis as { __kikoeruMockGates?: Record<string, { release: () => void }> };
		hooks.__kikoeruMockGates?.['GET /me/settings']?.release();
	});
	await expect(page.getByText('第 1 / 3 页 · 共 93 部')).toBeVisible();
	expect(await worksRequests(page)).toBe(1);
});
