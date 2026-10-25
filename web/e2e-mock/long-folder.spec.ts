import { expect, test } from '@playwright/test';

// The real library's largest folder holds about 700 files. Drawing them in
// one go took 400 ms on a phone-speed CPU, so the tree draws a first batch
// and adds the rest frame by frame (book: frontend.md, "长列表").
const FILES = 700;

test('a folder of 700 files opens with a first batch and fills in the rest', async ({ page }) => {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill('admin');
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
	await page.evaluate((n) => {
		(globalThis as { __kikoeruMockLargeFolder?: number }).__kikoeruMockLargeFolder = n;
	}, FILES);
	await page.getByRole('link', { name: /示例作品一/ }).click();
	await expect(page.getByRole('heading', { name: '示例作品一', level: 1 })).toBeVisible();

	const tracks = page.getByRole('list', { name: '曲目' });
	const folder = tracks.getByRole('button', { name: /^扫描图/ });
	// Counts the folder's rows in the frame right after the click.
	const firstFrame = await folder.evaluate(async (button) => {
		(button as HTMLElement).click();
		await new Promise((r) => requestAnimationFrame(r));
		return button.closest('li')!.querySelector('ul')!.children.length;
	});
	expect(firstFrame).toBeGreaterThan(0);
	expect(firstFrame).toBeLessThan(FILES);
	await expect(tracks.getByText('0700.jpg')).toBeAttached({ timeout: 15_000 });
	await expect(tracks.getByText('0001.jpg')).toBeVisible();
});
