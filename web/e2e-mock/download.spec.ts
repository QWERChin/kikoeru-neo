import { expect, test, type Page } from '@playwright/test';

// Downloads from the work detail page. 示例作品一 (work 12) resumes in
// mp3/SE有, so that folder is open when the page loads. The dev server
// answers the download endpoints with attachments, like the server.

async function signIn(page: Page) {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill('admin');
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
	await page.goto('/works/12');
	await expect(page.getByRole('heading', { name: '曲目' })).toBeVisible();
}

// The mock answers inside the page, so its own counter stands in for the network log.
const mediaTokenRequests = (page: Page) =>
	page.evaluate(
		() =>
			(globalThis as { __kikoeruMockRequests?: Record<string, number> }).__kikoeruMockRequests?.[
				'POST /auth/media-token'
			] ?? 0
	);

async function download(page: Page, button: string) {
	const [file] = await Promise.all([page.waitForEvent('download'), page.getByRole('button', { name: button }).click()]);
	return { name: file.suggestedFilename(), url: new URL(file.url()) };
}

test('a track downloads its own file with a media token issued for it', async ({ page }) => {
	await signIn(page);
	const before = await mediaTokenRequests(page);
	const file = await download(page, '下载 Tr02.mp3');
	expect(file.name).toBe('track-102.wav');
	expect(file.url.pathname).toBe('/api/tracks/102/download');
	expect(file.url.searchParams.get('token')).toBeTruthy();
	// The stored token may have expired while the page was open, and the
	// browser's download cannot retry a 401, so every download gets a new one.
	expect(await mediaTokenRequests(page)).toBe(before + 1);

	// Files the player skips can be downloaded too, missing ones cannot.
	await expect(page.getByRole('button', { name: '下载 cover.jpg' })).toBeVisible();
	await page.getByRole('button', { name: /^wav/ }).click();
	await expect(page.getByRole('button', { name: '下载 Tr01.wav' })).toBeVisible();
	await expect(page.getByText('Tr02.wav')).toBeVisible();
	await expect(page.getByRole('button', { name: '下载 Tr02.wav' })).toHaveCount(0);
});

test('a folder and the whole work download as zip files', async ({ page }) => {
	await signIn(page);
	const folder = await download(page, '下载文件夹 wav');
	expect(folder.name).toBe('work-12-wav.zip');
	expect(folder.url.pathname).toBe('/api/works/12/download');
	expect(folder.url.searchParams.get('group')).toBe('wav');
	expect(folder.url.searchParams.get('token')).toBeTruthy();
	// A nested folder is named by its whole path inside the work.
	const nested = await download(page, '下载文件夹 SE有');
	expect(nested.url.searchParams.get('group')).toBe('mp3/SE有');

	const whole = await download(page, '下载全部');
	expect(whole.name).toBe('work-12.zip');
	expect(whole.url.searchParams.has('group')).toBe(false);
});

test('a download that cannot get a media token says so and starts nothing', async ({ page }) => {
	await signIn(page);
	let downloads = 0;
	page.on('download', () => downloads++);
	await page.evaluate(() => {
		(globalThis as { __kikoeruMockFailOnce?: string }).__kikoeruMockFailOnce = 'POST /auth/media-token';
	});
	await page.getByRole('button', { name: '下载全部' }).click();
	await expect(page.getByRole('alert')).toHaveText('下载没有开始：模拟的服务器错误');
	expect(downloads).toBe(0);

	// The next try works and clears the message.
	await download(page, '下载全部');
	await expect(page.getByRole('alert')).toHaveCount(0);
});
