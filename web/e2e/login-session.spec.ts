import { expect, test, type BrowserContext, type Page } from '@playwright/test';
import { ADMIN, LISTENER } from './env';

async function submitLogin(page: Page, user = ADMIN) {
	await page.getByRole('textbox', { name: '用户名' }).fill(user.name);
	await page.getByLabel('密码').fill(user.password);
	await page.getByRole('button', { name: '登录', exact: true }).click();
}

async function openSettings(page: Page) {
	await page.locator('a[href="/settings"]:visible').first().click();
	await expect(page.getByRole('heading', { name: '设置', level: 1 })).toBeVisible();
}

async function signOut(page: Page) {
	await openSettings(page);
	await page.getByRole('button', { name: '退出登录', exact: true }).click();
	await expect(page).toHaveURL(/\/login$/);
}

async function holdMediaResponse(page: Page, status = 200) {
	let received!: (release: () => Promise<void>) => void;
	const pending = new Promise<() => Promise<void>>((resolve) => (received = resolve));
	await page.route('**/api/auth/media-token', async (route) => {
		const response = await route.fetch();
		expect(response.status()).toBe(200);
		received(async () => {
			const delivered = page.waitForResponse('**/api/auth/media-token');
			await route.fulfill(status === 200 ? { response } : {
				status,
				json: { error: { code: 'internal_error', message: 'Delayed media exchange failed' } }
			});
			await (await delivered).finished();
		});
	}, { times: 1 });
	return { pending };
}

async function coverStatus(page: Page) {
	// No Authorization header or URL token: only the browser's cover cookie
	// can authenticate this request to the real backend.
	return page.evaluate(async () => (await fetch('/api/works/1/cover')).status);
}

test('a late login media response cannot restore credentials after sign-out', async ({ page, context }) => {
	await page.goto('/login');
	await submitLogin(page);
	await expect(page).toHaveURL(/\/library$/);
	expect(await coverStatus(page)).toBe(200);
	await signOut(page);

	const held = await holdMediaResponse(page);
	await submitLogin(page);
	const release = await held.pending;
	// Going back within the SPA keeps the login request alive, while its
	// API token already allows the user to reach settings and sign out.
	await page.goBack();
	await expect(page).toHaveURL(/\/library$/);
	await signOut(page);
	await release();

	expect(await coverStatus(page)).toBe(401);
	expect(await page.evaluate(() => localStorage.getItem('kikoeru-neo.token'))).toBeNull();
	expect(await page.evaluate(() => localStorage.getItem('kikoeru-neo.media-token'))).toBeNull();
	expect((await context.cookies()).filter((cookie) => cookie.name.includes('kikoeru-media'))).toEqual([]);
	await expect(page).toHaveURL(/\/login$/);
});

for (const status of [200, 500]) {
	test(`a late login media response (${status}) leaves a newer account and its page alone`, async ({ page, context }) => {
		await page.goto('/login');
		await submitLogin(page);
		await expect(page).toHaveURL(/\/library$/);
		await signOut(page);

		const held = await holdMediaResponse(page, status);
		await submitLogin(page);
		const release = await held.pending;
		await page.goBack();
		await expect(page).toHaveURL(/\/library$/);
		await signOut(page);
		// Do not reload: the previous login must still be awaiting its media
		// response while another account signs in in the same JavaScript realm.
		await submitLogin(page, LISTENER);
		await expect(page).toHaveURL(/\/library$/);
		await openSettings(page);
		const credentials = await page.evaluate(() => ({
			api: localStorage.getItem('kikoeru-neo.token'),
			media: localStorage.getItem('kikoeru-neo.media-token')
		}));
		expect(credentials.api).not.toBeNull();
		expect(credentials.media).not.toBeNull();
		const cookies = await context.cookies();
		await release();

		expect(await coverStatus(page)).toBe(200);
		expect(await page.evaluate(() => ({
			api: localStorage.getItem('kikoeru-neo.token'),
			media: localStorage.getItem('kikoeru-neo.media-token')
		}))).toEqual(credentials);
		expect(await context.cookies()).toEqual(cookies);
		await expect(page).toHaveURL(/\/settings$/);
		await expect(page.getByText(LISTENER.name, { exact: true })).toBeVisible();
	});
}

test('a failed media exchange still completes the current login', async ({ page }) => {
	await page.goto('/login');
	const held = await holdMediaResponse(page, 500);
	await submitLogin(page);
	await (await held.pending)();
	await expect(page).toHaveURL(/\/library$/);
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
	expect(await page.evaluate(() => localStorage.getItem('kikoeru-neo.token'))).not.toBeNull();
	expect(await page.evaluate(() => localStorage.getItem('kikoeru-neo.media-token'))).toBeNull();
	expect(await coverStatus(page)).toBe(401);
});

async function signInWithTokenStorageFailure(page: Page, context: BrowserContext) {
	await page.goto('/login');
	await expect(page.getByRole('button', { name: '登录', exact: true })).toBeVisible();
	// This tab stays on the login page while another account signs in.
	const oldAccount = await context.newPage();
	await oldAccount.goto('/login');
	await submitLogin(oldAccount);
	await expect(oldAccount).toHaveURL(/\/library$/);
	const oldToken = await oldAccount.evaluate(() => localStorage.getItem('kikoeru-neo.token'));
	expect(oldToken).not.toBeNull();
	await oldAccount.close();
	// Quota exhaustion can reject a larger replacement token while reads
	// and removals still work. Leave the previous account in storage.
	await page.evaluate(() => {
		const setItem = Storage.prototype.setItem;
		Storage.prototype.setItem = function (key, value) {
			if (key === 'kikoeru-neo.token') throw new DOMException('Storage is full', 'QuotaExceededError');
			setItem.call(this, key, value);
		};
	});
	await submitLogin(page, LISTENER);
	await expect(page).toHaveURL(/\/library$/);
	await openSettings(page);
	await expect(page.getByText(LISTENER.name, { exact: true })).toBeVisible();
	expect(await page.evaluate(() => localStorage.getItem('kikoeru-neo.token'))).toBe(oldToken);
	expect(await coverStatus(page)).toBe(200);
	return oldToken;
}

test('sign-out clears old credentials and cover access after a token storage failure', async ({ page, context }) => {
	await signInWithTokenStorageFailure(page, context);
	await page.getByRole('button', { name: '退出登录', exact: true }).click();
	await expect(page).toHaveURL(/\/login$/);
	expect(await coverStatus(page)).toBe(401);
	expect(await page.evaluate(() => localStorage.getItem('kikoeru-neo.token'))).toBeNull();
	expect(await page.evaluate(() => localStorage.getItem('kikoeru-neo.media-token'))).toBeNull();
	expect((await context.cookies()).filter((cookie) => cookie.name.includes('kikoeru-media'))).toEqual([]);
	await page.goto('/library');
	await expect(page).toHaveURL(/\/login$/);
});

test('a memory-only session preserves a newer login in another tab', async ({ page, context }) => {
	const oldToken = await signInWithTokenStorageFailure(page, context);
	const newerTab = await context.newPage();
	await newerTab.goto('/library');
	await openSettings(newerTab);
	// Rotate the old account's token version so its new login has a
	// different token even when both logins happen within one second.
	await newerTab.getByRole('button', { name: '退出所有设备' }).click();
	await newerTab.getByRole('group', { name: '退出所有设备' }).getByRole('button', { name: '全部退出' }).click();
	await expect(newerTab).toHaveURL(/\/login$/);
	await submitLogin(newerTab);
	await expect(newerTab).toHaveURL(/\/library$/);
	const credentials = await newerTab.evaluate(() => ({
		api: localStorage.getItem('kikoeru-neo.token'),
		media: localStorage.getItem('kikoeru-neo.media-token')
	}));
	expect(credentials.api).not.toBeNull();
	expect(credentials.api).not.toBe(oldToken);
	expect(credentials.media).not.toBeNull();
	const cookies = await context.cookies();

	await page.getByRole('button', { name: '退出登录', exact: true }).click();
	await expect(page).toHaveURL(/\/login$/);
	expect(await page.evaluate(() => ({
		api: localStorage.getItem('kikoeru-neo.token'),
		media: localStorage.getItem('kikoeru-neo.media-token')
	}))).toEqual(credentials);
	expect(await context.cookies()).toEqual(cookies);
	expect(await coverStatus(newerTab)).toBe(200);
	await newerTab.reload();
	await expect(newerTab).toHaveURL(/\/library$/);
	await openSettings(newerTab);
	await expect(newerTab.getByText(ADMIN.name, { exact: true })).toBeVisible();
});

test('a revoked tab cannot clear credentials from a newer login in another tab', async ({ page, context }) => {
	await page.goto('/login');
	await submitLogin(page);
	await expect(page).toHaveURL(/\/library$/);
	const oldToken = await page.evaluate(() => localStorage.getItem('kikoeru-neo.token'));
	const oldTab = await context.newPage();
	await oldTab.goto('/library');
	await expect(oldTab.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
	await expect(oldTab.getByRole('link', { name: '下一页' })).toBeVisible();

	await openSettings(page);
	await page.getByRole('button', { name: '退出所有设备' }).click();
	await page.getByRole('group', { name: '退出所有设备' }).getByRole('button', { name: '全部退出' }).click();
	await expect(page).toHaveURL(/\/login$/);
	await submitLogin(page);
	await expect(page).toHaveURL(/\/library$/);
	const credentials = await page.evaluate(() => ({
		api: localStorage.getItem('kikoeru-neo.token'),
		media: localStorage.getItem('kikoeru-neo.media-token')
	}));
	expect(credentials.api).not.toBeNull();
	expect(credentials.api).not.toBe(oldToken);
	expect(credentials.media).not.toBeNull();
	const cookies = await context.cookies();
	expect(await coverStatus(page)).toBe(200);

	// SPA pagination keeps the second tab's revoked token in memory.
	const rejected = oldTab.waitForResponse((response) =>
		response.url().includes('/api/works?') && response.status() === 401
	);
	await oldTab.getByRole('link', { name: '下一页' }).click();
	expect((await rejected).request().headers().authorization).toBe(`Bearer ${oldToken}`);
	await expect(oldTab).toHaveURL(/\/login$/);

	expect(await page.evaluate(() => ({
		api: localStorage.getItem('kikoeru-neo.token'),
		media: localStorage.getItem('kikoeru-neo.media-token')
	}))).toEqual(credentials);
	expect(await context.cookies()).toEqual(cookies);
	expect(await coverStatus(page)).toBe(200);
	await page.reload();
	await expect(page).toHaveURL(/\/library$/);
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
});
