import { expect, test, type Page } from '@playwright/test';
import { stubShell } from './shell-stub';

// The 网络代理 section of the settings page in mock mode. The mock starts
// with no proxy saved, and a proxy on the host unreachable.test fails the
// check (src/lib/api/mock-settings.ts). A reload resets the mock, so the
// tests move between pages by clicking links.

const DESKTOP_NOTE = '桌面版不会自动使用系统代理';

async function signIn(page: Page, name = 'admin') {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill(name);
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

async function navigate(page: Page, label: string) {
	await page.getByRole('navigation', { name: '主导航' }).getByRole('link', { name: label, exact: true }).click();
	await expect(page.getByRole('heading', { name: label, level: 1 })).toBeVisible();
}

const section = (page: Page) => page.getByRole('region', { name: '网络代理' });
const field = (page: Page) => section(page).getByRole('textbox', { name: '元数据代理' });
const button = (page: Page, name: string) => section(page).getByRole('button', { name });

const requests = (page: Page, key: string) =>
	page.evaluate(
		(k) => (globalThis as { __kikoeruMockRequests?: Record<string, number> }).__kikoeruMockRequests?.[k] ?? 0,
		key
	);

async function openSection(page: Page) {
	await signIn(page);
	await navigate(page, '设置');
	await expect(field(page)).toBeVisible();
}

test('loads the saved proxy and saves the normalised form', async ({ page }) => {
	await openSection(page);
	await expect(field(page)).toHaveValue('');
	await expect(field(page)).toHaveAttribute('placeholder', 'http://127.0.0.1:7890');
	await expect(section(page)).toContainText('留空时按服务器的环境变量 HTTPS_PROXY 决定');
	await expect(section(page)).not.toContainText(DESKTOP_NOTE);
	await expect(button(page, '保存')).toBeDisabled();

	await field(page).fill('HTTP://LocalHost:7890/');
	await button(page, '保存').click();
	await expect(field(page)).toHaveValue('http://localhost:7890');
	await expect(section(page)).toContainText('已保存，之后的元数据请求会走这个代理。');
	await expect(button(page, '保存')).toBeDisabled();

	// Coming back reads the setting from the server again.
	await navigate(page, '资料库');
	await navigate(page, '设置');
	await expect(field(page)).toHaveValue('http://localhost:7890');
	expect(await requests(page, 'GET /admin/settings')).toBe(2);

	await field(page).fill('');
	await button(page, '保存').click();
	await expect(section(page)).toContainText('已清除，之后按环境变量决定。');
	await expect(field(page)).toHaveValue('');
	await expect(button(page, '保存')).toBeDisabled();
});

test('keeps an invalid proxy in the field and says what is wrong', async ({ page }) => {
	await openSection(page);
	await field(page).fill('http://user:secret@127.0.0.1:7890');
	await field(page).press('Enter');
	await expect(section(page).getByRole('alert')).toHaveText('没有保存：代理地址不能带用户名和密码');
	await expect(field(page)).toHaveValue('http://user:secret@127.0.0.1:7890');
	await expect(button(page, '保存')).toBeEnabled();

	// Editing the field clears the message about the old value.
	await field(page).press('Backspace');
	await expect(section(page).getByRole('alert')).toHaveCount(0);
});

test('tests the typed proxy without saving it', async ({ page }) => {
	await openSection(page);
	await field(page).fill('socks5://127.0.0.1:1080');
	await page.evaluate(() => {
		(globalThis as { __kikoeruMockHoldMs?: Record<string, number> }).__kikoeruMockHoldMs = {
			'POST /admin/settings/proxy-check': 1_000
		};
	});
	await button(page, '测试连接').click();
	await expect(button(page, '测试中…')).toBeDisabled();
	await expect(section(page)).toContainText('连接正常：DLsite 回答了 HTTP 200，用时 380 毫秒。');
	await expect(button(page, '测试连接')).toBeEnabled();
	expect(await requests(page, 'PUT /admin/settings')).toBe(0);
	await expect(button(page, '保存')).toBeEnabled();
});

test('drops the result of a check for a value that was edited meanwhile', async ({ page }) => {
	await openSection(page);
	await field(page).fill('http://127.0.0.1:7890');
	await page.evaluate(() => {
		(globalThis as { __kikoeruMockHoldMs?: Record<string, number> }).__kikoeruMockHoldMs = {
			'POST /admin/settings/proxy-check': 1_000
		};
	});
	await button(page, '测试连接').click();
	await expect(button(page, '测试中…')).toBeDisabled();
	await field(page).fill('http://unreachable.test:1');

	await expect(button(page, '测试连接')).toBeEnabled();
	expect(await requests(page, 'POST /admin/settings/proxy-check')).toBe(1);
	await expect(section(page)).not.toContainText('连接正常');
	await expect(section(page).getByRole('alert')).toHaveCount(0);
});

test('says nothing about a save once the field has been edited meanwhile', async ({ page }) => {
	await openSection(page);
	await field(page).fill('http://127.0.0.1:7890');
	await page.evaluate(() => {
		(globalThis as { __kikoeruMockHoldMs?: Record<string, number> }).__kikoeruMockHoldMs = {
			'PUT /admin/settings': 1_000
		};
	});
	await button(page, '保存').click();
	await expect(button(page, '保存')).toBeDisabled();
	await field(page).fill('http://127.0.0.1:7891');

	// 保存 comes back once the save has landed, enabled for the value now typed.
	await expect(button(page, '保存')).toBeEnabled();
	expect(await requests(page, 'PUT /admin/settings')).toBe(1);
	await expect(field(page)).toHaveValue('http://127.0.0.1:7891');
	await expect(section(page)).not.toContainText('已保存');
	await field(page).fill('http://127.0.0.1:7890');
	await expect(button(page, '保存')).toBeDisabled();
});

test('shows why the proxy could not connect', async ({ page }) => {
	await openSection(page);
	await field(page).fill('http://unreachable.test:1');
	await button(page, '测试连接').click();
	await expect(section(page).getByRole('alert')).toHaveText('连不上 DLsite：tcp connect error: Connection refused (os error 111)');
	await expect(section(page)).not.toContainText('连接正常');
});

test('says when the setting could not be loaded', async ({ page }) => {
	await signIn(page);
	await page.evaluate(() => {
		(globalThis as { __kikoeruMockFailOnce?: string | null }).__kikoeruMockFailOnce = 'GET /admin/settings';
	});
	await navigate(page, '设置');
	await expect(section(page).getByRole('alert')).toHaveText('代理设置加载失败：模拟的服务器错误');
	await expect(field(page)).toHaveCount(0);
});

test('a listener neither sees nor loads the setting', async ({ page }) => {
	await signIn(page, 'listener');
	await navigate(page, '设置');
	await expect(page.getByRole('button', { name: '退出登录' })).toBeVisible();
	await expect(section(page)).toHaveCount(0);
	expect(await requests(page, 'GET /admin/settings')).toBe(0);
});

test('the desktop app explains that it ignores the system proxy', async ({ page }) => {
	await stubShell(page, {
		kind: 'local',
		state: { mode: 'local', server_url: null, last_error: null },
		token: 'shell-issued-token'
	});
	await page.goto('/');
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
	await navigate(page, '设置');
	await expect(section(page)).toContainText(`${DESKTOP_NOTE}，需要走代理时请在这里填写。`);
});
