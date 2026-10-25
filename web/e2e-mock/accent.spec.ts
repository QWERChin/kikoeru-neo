import { expect, test, type Page } from '@playwright/test';

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

const rootAccent = (page: Page) => page.evaluate(() => document.documentElement.dataset.accent ?? null);
const accentText = (page: Page) =>
	page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--accent-text').trim());
const putBodies = (page: Page) =>
	page.evaluate(
		() => (globalThis as { __kikoeruMockRequests?: Record<string, number> }).__kikoeruMockRequests?.['PUT /me/settings'] ?? 0
	);

test('picking an accent recolours the app, saves it and survives a reload', async ({ page }) => {
	await signIn(page);
	await expect.poll(() => rootAccent(page)).toBe('blue');
	await openSettings(page);
	const group = page.getByRole('radiogroup', { name: '强调色' });
	await expect(group.getByRole('radio')).toHaveCount(8);
	await expect(group.getByRole('radio', { name: '蓝色' })).toHaveAttribute('aria-checked', 'true');
	const blue = await accentText(page);

	await group.getByRole('radio', { name: '紫色' }).click();
	await expect(group.getByRole('radio', { name: '紫色' })).toHaveAttribute('aria-checked', 'true');
	await expect.poll(() => rootAccent(page)).toBe('purple');
	expect(await accentText(page)).not.toBe(blue);
	// The rating pill on covers is dark in both themes and takes the accent's dark tint.
	expect(
		await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--pill-star').trim())
	).toBe('#c66df3');
	await expect.poll(() => putBodies(page)).toBe(1);

	// After a reload the remembered accent paints the page, also when the
	// settings cannot be read. The mock forgets them on reload, so its answer
	// would say blue.
	await page.addInitScript(() => {
		(globalThis as { __kikoeruMockFailOnce?: string }).__kikoeruMockFailOnce = 'GET /me/settings';
	});
	await page.reload();
	await expect(page.getByRole('heading', { name: '设置', level: 1 })).toBeVisible();
	await expect(page.getByRole('radiogroup', { name: '强调色' })).toBeVisible();
	expect(await rootAccent(page)).toBe('purple');
	// Without the account's settings nothing can be saved, because a PUT
	// replaces all of them. The swatches show what the page uses.
	const swatches = page.getByRole('radiogroup', { name: '强调色' });
	await expect(swatches.getByRole('radio', { name: '紫色' })).toHaveAttribute('aria-checked', 'true');
	await expect(swatches.getByRole('radio', { name: '蓝色' })).toBeDisabled();
	await expect(page.getByRole('switch', { name: '音量均衡' })).toBeDisabled();
});

test('every swatch draws in its own colour whatever the page accent is', async ({ page }) => {
	await signIn(page);
	await openSettings(page);
	const group = page.getByRole('radiogroup', { name: '强调色' });
	await group.getByRole('radio', { name: '红色' }).click();
	const colours = await group
		.getByRole('radio')
		.evaluateAll((swatches) => swatches.map((s) => getComputedStyle(s.querySelector('span')!).backgroundColor));
	expect(new Set(colours).size).toBe(8);
});
