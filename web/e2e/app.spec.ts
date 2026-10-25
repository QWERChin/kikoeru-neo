import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { ADMIN, LISTENER, PAGE_SIZE, WORK_COUNT } from './env';

const PAGE_COUNT = Math.ceil(WORK_COUNT / PAGE_SIZE);

function workTitle(n: number): string {
	return `E2E 合成作品 ${String(n).padStart(2, '0')}`;
}

async function signIn(page: Page, user: { name: string; password: string }) {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill(user.name);
	await page.getByLabel('密码').fill(user.password);
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page).toHaveURL(/\/library$/);
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

function card(page: Page, n: number) {
	return page.getByRole('article').filter({ has: page.getByRole('heading', { name: workTitle(n) }) });
}

test('login shows the first page of the real library', async ({ page }, testInfo) => {
	await signIn(page, ADMIN);
	await expect(page.getByRole('article')).toHaveCount(PAGE_SIZE);
	await expect(page.getByText(`第 1 / ${PAGE_COUNT} 页 · 共 ${WORK_COUNT} 部`)).toBeVisible();
	// Newest first, and none of the mock samples.
	await expect(page.getByRole('article').first()).toContainText(workTitle(WORK_COUNT));
	await expect(page.getByText('示例作品一')).toHaveCount(0);

	const dir = process.env.KIKOERU_E2E_SCREENSHOTS;
	if (dir) {
		mkdirSync(dir, { recursive: true });
		await page.screenshot({ path: join(dir, `e2e-library-${testInfo.project.name}.png`) });
	}
});

test('pager moves through the pages', async ({ page }) => {
	await signIn(page, ADMIN);
	await page.getByRole('link', { name: '下一页' }).click();
	await expect(page).toHaveURL(/\/library\?page=2$/);
	await expect(page.getByRole('article')).toHaveCount(WORK_COUNT - PAGE_SIZE);
	await expect(page.getByRole('article').last()).toContainText(workTitle(1));
	await expect(page.getByRole('button', { name: '下一页' })).toBeDisabled();

	await page.reload();
	await expect(page.getByText(`第 2 / ${PAGE_COUNT} 页`)).toBeVisible();
	await page.getByRole('link', { name: '上一页' }).click();
	await expect(page).toHaveURL(/\/library$/);
});

test('covers show favorite, finished and listening icons', async ({ page }) => {
	await signIn(page, ADMIN);
	const n = WORK_COUNT;
	const expected: [number, string][] = [
		[n, '已收藏'],
		[n - 1, '在听'],
		[n - 2, '听过'],
		[n - 3, '听过，重听中'],
		[n - 4, '已收藏，听过'],
		[n - 5, '在听']
	];
	for (const [work, label] of expected) {
		await expect(card(page, work).getByRole('img', { name: label, exact: true })).toBeVisible();
	}
	await expect(card(page, n - 6).getByRole('img')).toHaveCount(0);
});

test('deep links load the app directly', async ({ page }) => {
	const response = await page.goto('/library?page=2');
	expect(response?.status()).toBe(200);
	expect(response?.headers()['content-type']).toContain('text/html');
	// Without a session the app itself redirects, which proves it loaded.
	await expect(page).toHaveURL(/\/login$/);

	await signIn(page, ADMIN);
	const deepLinks: [string, string][] = [
		['/settings', '设置'],
		['/library?page=2', '资料库']
	];
	for (const [path, heading] of deepLinks) {
		const r = await page.goto(path);
		expect(r?.status()).toBe(200);
		await expect(page.getByRole('heading', { name: heading, level: 1 })).toBeVisible();
	}
	await expect(page.getByText(`第 2 / ${PAGE_COUNT} 页`)).toBeVisible();
});

test('sign out returns to login and forgets the session', async ({ page }) => {
	await signIn(page, ADMIN);
	await page.getByRole('navigation', { name: '主导航' }).getByRole('link', { name: '设置' }).click();
	await expect(page.getByRole('heading', { name: '设置', level: 1 })).toBeVisible();
	await page.getByRole('button', { name: '退出登录' }).click();
	await expect(page).toHaveURL(/\/login$/);
	await page.goto('/library');
	await expect(page).toHaveURL(/\/login$/);
});

test('unknown API paths and missing files are real 404s', async ({ request }) => {
	for (const path of ['/api/nope', '/api', '/_app/immutable/missing.js', '/favicon-nope.png']) {
		const response = await request.get(path);
		expect(response.status(), path).toBe(404);
		expect(response.headers()['content-type'], path).toBe('application/json');
		expect((await response.json()).error.code, path).toBe('not_found');
	}
	const asset = await request.get('/favicon.svg');
	expect(asset.status()).toBe(200);
	expect(asset.headers()['content-type']).toBe('image/svg+xml');
});

test('the settings page changes the password and signs out', async ({ page, request }) => {
	await signIn(page, LISTENER);
	await page.locator('a[href="/settings"]:visible').first().click();
	await page.getByRole('button', { name: '修改密码' }).click();
	const form = page.getByRole('form', { name: '修改密码' });
	const changed = `${LISTENER.password} from settings`;

	await form.getByLabel('当前密码').fill('not the password');
	await form.getByLabel('新密码', { exact: true }).fill(changed);
	await form.getByLabel('确认新密码').fill(changed);
	await form.getByRole('button', { name: '修改', exact: true }).click();
	// The server answers 401 here too, but the session must survive it.
	await expect(form.getByRole('alert')).toHaveText('当前密码不对。');
	await expect(page).toHaveURL(/\/settings$/);

	await form.getByLabel('当前密码').fill(LISTENER.password);
	await form.getByRole('button', { name: '修改', exact: true }).click();
	try {
		await expect(page).toHaveURL(/\/login$/);
		await expect(page.getByRole('status')).toHaveText('密码已经改好，请用新密码登录。');
		const old = await request.post('/api/auth/login', {
			data: { name: LISTENER.name, password: LISTENER.password }
		});
		expect(old.status()).toBe(401);
		await signIn(page, { name: LISTENER.name, password: changed });
	} finally {
		// Other tests sign in with the original password. The change may not
		// have happened if the test failed early.
		const login = await request.post('/api/auth/login', {
			data: { name: LISTENER.name, password: changed }
		});
		if (login.ok()) {
			const { token } = await login.json();
			const back = await request.put('/api/auth/password', {
				headers: { Authorization: `Bearer ${token}` },
				data: { current_password: changed, new_password: LISTENER.password }
			});
			expect(back.status()).toBe(204);
		}
	}
});

test('a revoked token sends the browser back to login', async ({ page, request }) => {
	await signIn(page, LISTENER);
	await expect(page.getByRole('article')).toHaveCount(PAGE_SIZE);

	const changePassword = async (from: string, to: string) => {
		const login = await request.post('/api/auth/login', {
			data: { name: LISTENER.name, password: from }
		});
		expect(login.status()).toBe(200);
		const { token } = await login.json();
		const change = await request.put('/api/auth/password', {
			headers: { Authorization: `Bearer ${token}` },
			data: { current_password: from, new_password: to }
		});
		expect(change.status()).toBe(204);
	};

	const temporary = `${LISTENER.password} changed`;
	await changePassword(LISTENER.password, temporary);
	try {
		// The next API call carries the revoked token and gets a 401.
		await page.getByRole('link', { name: '下一页' }).click();
		await expect(page).toHaveURL(/\/login$/);
		await expect(page.getByRole('button', { name: '登录' })).toBeVisible();
		expect(await page.evaluate(() => localStorage.getItem('kikoeru-neo.token'))).toBeNull();
	} finally {
		// The other project runs this test again with the original password.
		await changePassword(temporary, LISTENER.password);
	}
});
