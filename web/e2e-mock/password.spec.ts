import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page, type TestInfo } from '@playwright/test';

async function signIn(page: Page) {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill('admin');
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

async function openForm(page: Page) {
	await page.locator('a[href="/settings"]:visible').first().click();
	await expect(page.getByRole('heading', { name: '设置', level: 1 })).toBeVisible();
	await page.getByRole('button', { name: '修改密码' }).click();
	return page.getByRole('form', { name: '修改密码' });
}

async function shot(page: Page, testInfo: TestInfo, name: string) {
	const dir = process.env.KIKOERU_MOCK_SCREENSHOTS;
	if (!dir) return;
	mkdirSync(dir, { recursive: true });
	await page.screenshot({ path: join(dir, `password-${name}-${testInfo.project.name}.png`) });
}

const requests = (page: Page) =>
	page.evaluate(
		() =>
			(globalThis as { __kikoeruMockRequests?: Record<string, number> }).__kikoeruMockRequests?.[
				'PUT /auth/password'
			] ?? 0
	);

test('the form checks the new password before asking the server', async ({ page }, testInfo) => {
	await signIn(page);
	const form = await openForm(page);
	await expect(form.getByLabel('当前密码')).toBeFocused();
	await expect(form).toContainText('至少 8 个字符');
	await shot(page, testInfo, 'form');

	await form.getByLabel('当前密码').fill('old password');
	await form.getByLabel('新密码', { exact: true }).fill('short');
	await form.getByLabel('确认新密码').fill('other');
	await form.getByRole('button', { name: '修改', exact: true }).click();
	await expect(form).toContainText('新密码至少要 8 个字符');
	await expect(form).toContainText('两次输入的新密码不一样');
	await expect(form.getByLabel('新密码', { exact: true })).toHaveAttribute('aria-invalid', 'true');
	await shot(page, testInfo, 'invalid');
	expect(await requests(page)).toBe(0);

	// Cancelling forgets what was typed and puts focus back on the row.
	await form.getByRole('button', { name: '取消' }).click();
	await expect(form).toBeHidden();
	await expect(page.getByRole('button', { name: '修改密码' })).toBeFocused();
	await page.getByRole('button', { name: '修改密码' }).click();
	await expect(form.getByLabel('当前密码')).toHaveValue('');
	await expect(form.getByLabel('新密码', { exact: true })).toHaveValue('');
	await expect(form).not.toContainText('两次输入的新密码不一样');
});

test('a wrong current password stays on the form and keeps the session', async ({ page }) => {
	await signIn(page);
	const form = await openForm(page);
	await form.getByLabel('当前密码').fill('wrong password');
	await form.getByLabel('新密码', { exact: true }).fill('a new password');
	await form.getByLabel('确认新密码').fill('a new password');
	await form.getByRole('button', { name: '修改', exact: true }).click();

	await expect(form.getByRole('alert')).toHaveText('当前密码不对。');
	await expect(form.getByLabel('当前密码')).toHaveValue('');
	await expect(form.getByLabel('当前密码')).toBeFocused();
	await expect(form.getByLabel('新密码', { exact: true })).toHaveValue('a new password');
	expect(await requests(page)).toBe(1);
	// The 401 was about the password in the body, not the token.
	await page.reload();
	await expect(page.getByRole('heading', { name: '设置', level: 1 })).toBeVisible();
	await expect(page).toHaveURL(/\/settings$/);
});

test('a server error keeps the form and says why', async ({ page }) => {
	await signIn(page);
	const form = await openForm(page);
	await page.evaluate(() => {
		(globalThis as { __kikoeruMockFailOnce?: string }).__kikoeruMockFailOnce = 'PUT /auth/password';
	});
	await form.getByLabel('当前密码').fill('old password');
	await form.getByLabel('新密码', { exact: true }).fill('a new password');
	await form.getByLabel('确认新密码').fill('a new password');
	await form.getByRole('button', { name: '修改', exact: true }).click();
	await expect(form.getByRole('alert')).toHaveText('没有修改：模拟的服务器错误');
	await expect(form.getByLabel('当前密码')).toHaveValue('old password');
	await expect(page).toHaveURL(/\/settings$/);
});

test('a changed password signs this device out and the login page says so', async ({ page }, testInfo) => {
	await signIn(page);
	const form = await openForm(page);
	await form.getByLabel('当前密码').fill('old password');
	await form.getByLabel('新密码', { exact: true }).fill('a new password');
	await form.getByLabel('确认新密码').fill('a new password');
	await form.getByRole('button', { name: '修改', exact: true }).click();

	await expect(page).toHaveURL(/\/login$/);
	await expect(page.getByRole('status')).toHaveText('密码已经改好，请用新密码登录。');
	expect(await page.evaluate(() => localStorage.getItem('kikoeru-neo.token'))).toBeNull();
	await shot(page, testInfo, 'signed-out');

	// The notice goes away once a sign-in attempt fails. The mock refuses `locked`.
	await page.getByRole('textbox', { name: '用户名' }).fill('locked');
	await page.getByLabel('密码').fill('a new password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('alert')).toBeVisible();
	await expect(page.getByRole('status')).toBeHidden();
});
