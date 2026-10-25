import { expect, test, type Page } from '@playwright/test';

async function signIn(page: Page) {
	await page.goto('/login');
	await fillSignIn(page);
}

/** Signs in on the login page already shown, keeping the page's JavaScript. */
async function fillSignIn(page: Page) {
	await page.getByRole('textbox', { name: '用户名' }).fill('admin');
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

async function openSettings(page: Page) {
	await page.locator('a[href="/settings"]:visible').first().click();
	await expect(page.getByRole('heading', { name: '设置', level: 1 })).toBeVisible();
}

const requests = (page: Page) =>
	page.evaluate(
		() =>
			(globalThis as { __kikoeruMockRequests?: Record<string, number> }).__kikoeruMockRequests?.[
				'POST /auth/logout-all'
			] ?? 0
	);

test('logging out everywhere asks first, then signs this device out too', async ({ page }) => {
	await signIn(page);
	await openSettings(page);
	await page.getByRole('button', { name: '退出所有设备' }).click();
	const confirm = page.getByRole('group', { name: '退出所有设备' });
	await expect(confirm).toContainText('所有设备上的登录都会失效');
	await confirm.getByRole('button', { name: '取消' }).click();
	await expect(confirm).toBeHidden();
	expect(await requests(page)).toBe(0);

	await page.getByRole('button', { name: '退出所有设备' }).click();
	await page.getByRole('group', { name: '退出所有设备' }).getByRole('button', { name: '全部退出' }).click();
	await expect(page).toHaveURL(/\/login$/);
	expect(await requests(page)).toBe(1);
	// The stored session is gone, so a reload stays on the login page.
	await page.reload();
	await expect(page.getByRole('button', { name: '登录' })).toBeVisible();
});

test('a failed logout-all keeps the user signed in and says why', async ({ page }) => {
	await signIn(page);
	await openSettings(page);
	await page.evaluate(() => {
		(globalThis as { __kikoeruMockFailOnce?: string }).__kikoeruMockFailOnce = 'POST /auth/logout-all';
	});
	await page.getByRole('button', { name: '退出所有设备' }).click();
	await page.getByRole('group', { name: '退出所有设备' }).getByRole('button', { name: '全部退出' }).click();
	await expect(page.getByRole('alert')).toHaveText('没有退出：模拟的服务器错误');
	await expect(page.getByRole('heading', { name: '设置', level: 1 })).toBeVisible();
});

test('a slow logout-all answer leaves a newer sign-in alone', async ({ page }) => {
	await signIn(page);
	await openSettings(page);
	// The logout-all answer waits until the test lets it go.
	await page.evaluate(() => {
		const hooks = globalThis as {
			__kikoeruMockWait?: Record<string, Promise<void>>;
			__releaseLogoutAll?: () => void;
		};
		hooks.__kikoeruMockWait = {
			'POST /auth/logout-all': new Promise((resolve) => (hooks.__releaseLogoutAll = resolve))
		};
	});
	await page.getByRole('button', { name: '退出所有设备' }).click();
	await page.getByRole('group', { name: '退出所有设备' }).getByRole('button', { name: '全部退出' }).click();
	await page.getByRole('button', { name: '退出登录', exact: true }).click();
	await expect(page).toHaveURL(/\/login$/);
	// The sign-out moved within the app, so the old request is still pending
	// in this page. page.goto would reload it and drop that request.
	await fillSignIn(page);
	// Only now does the old request answer. The new session must survive it.
	await page.evaluate(() => (globalThis as { __releaseLogoutAll?: () => void }).__releaseLogoutAll?.());
	await page.waitForTimeout(500);
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
	await expect(page).not.toHaveURL(/\/login$/);
});
