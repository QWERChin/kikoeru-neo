import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { expect, test, type Page, type TestInfo } from '@playwright/test';

// The mock seeds two reviews from contract/reviews/: 示例作品一 (work 12)
// with 4 stars and a text, and the missing 示例作品四 (work 15) with a text
// only (src/lib/api/mock-reviews.ts). 示例作品二 (work 13) has none.

async function signIn(page: Page) {
	await page.goto('/login');
	await page.getByRole('textbox', { name: '用户名' }).fill('admin');
	await page.getByLabel('密码').fill('any password');
	await page.getByRole('button', { name: '登录' }).click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

async function shot(page: Page, testInfo: TestInfo, name: string) {
	const dir = process.env.KIKOERU_MOCK_SCREENSHOTS;
	if (!dir) return;
	mkdirSync(dir, { recursive: true });
	await page.waitForLoadState('networkidle');
	await page.screenshot({ path: join(dir, `reviews-${name}-${testInfo.project.name}.png`) });
}

const card = (page: Page, title: string) =>
	page.getByRole('article').filter({ has: page.getByRole('heading', { name: title, exact: true }) });

// Client-side navigation only, because a reload would reset the mock's reviews.
async function openLibrary(page: Page) {
	await page.locator('a[href="/library"]:visible').first().click();
	await expect(page.getByRole('heading', { name: '资料库', level: 1 })).toBeVisible();
}

async function openWork(page: Page, title: string, id: number) {
	await card(page, title).getByRole('link').click();
	await expect(page).toHaveURL(new RegExp(`/works/${id}$`));
	await expect(page.getByRole('heading', { name: title, level: 1 })).toBeVisible();
}

/** Phones reach the page from 设置, desktops from the sidebar. */
async function openReviews(page: Page, testInfo: TestInfo) {
	if (testInfo.project.name === 'phone') {
		await page.locator('a[href="/settings"]:visible').first().click();
		await expect(page.getByRole('heading', { name: '设置', level: 1 })).toBeVisible();
		await page.getByRole('link', { name: '我的评价' }).click();
	} else {
		await page.getByRole('navigation', { name: '主导航' }).getByRole('link', { name: '我的评价' }).click();
	}
	await expect(page.getByRole('heading', { name: '我的评价', level: 1 })).toBeVisible();
}

const reviewGroup = (page: Page) => page.getByRole('region', { name: '我的评价' });
const star = (page: Page, n: number) => reviewGroup(page).getByRole('button', { name: `评 ${n} 星` });
const sheet = (page: Page) => page.getByRole('dialog', { name: '我的评价' });
const requests = (page: Page, key: string) =>
	page.evaluate(
		(k) => (globalThis as { __kikoeruMockRequests?: Record<string, number> }).__kikoeruMockRequests?.[k] ?? 0,
		key
	);

test('the phone tab bar keeps five items', async ({ page }, testInfo) => {
	test.skip(testInfo.project.name !== 'phone', 'Only phones have the tab bar.');
	await signIn(page);
	await expect(page.getByRole('navigation', { name: '主导航' }).getByRole('link')).toHaveCount(5);
});

test('a rating saved on the detail page shows on the card, and tapping it again clears it', async ({ page }, testInfo) => {
	await signIn(page);
	await expect(card(page, '示例作品一').getByRole('img', { name: '我的评分 4 星' })).toBeVisible();
	await expect(card(page, '示例作品二').getByRole('img', { name: /我的评分/ })).toHaveCount(0);

	await openWork(page, '示例作品二', 13);
	await expect(reviewGroup(page).getByRole('button', { name: '写短评' })).toBeVisible();
	await star(page, 3).click();
	await expect(star(page, 3)).toHaveAttribute('aria-pressed', 'true');
	// The stars are disabled until the server answers.
	await expect(star(page, 3)).toBeEnabled();
	await expect(star(page, 4)).toHaveAttribute('aria-pressed', 'false');
	await shot(page, testInfo, 'detail');

	await page.getByRole('button', { name: '返回' }).click();
	await expect(card(page, '示例作品二').getByRole('img', { name: '我的评分 3 星' })).toBeVisible();
	await shot(page, testInfo, 'card');

	await openWork(page, '示例作品二', 13);
	await expect(star(page, 3)).toHaveAttribute('aria-pressed', 'true');
	await star(page, 3).click();
	await expect(star(page, 3)).toHaveAttribute('aria-pressed', 'false');
	await expect(star(page, 3)).toBeEnabled();
	// Without a text, clearing the rating leaves nothing, so it is a DELETE.
	expect(await requests(page, 'DELETE /me/reviews/13')).toBe(1);

	await openLibrary(page);
	await expect(card(page, '示例作品二')).toBeVisible();
	await expect(card(page, '示例作品二').getByRole('img', { name: /我的评分/ })).toHaveCount(0);
});

test('clearing the rating of a review with text keeps the text', async ({ page }) => {
	await signIn(page);
	await openWork(page, '示例作品一', 12);
	await expect(star(page, 4)).toHaveAttribute('aria-pressed', 'true');
	await star(page, 4).click();
	await expect(star(page, 4)).toHaveAttribute('aria-pressed', 'false');
	await expect(star(page, 4)).toBeEnabled();
	await expect(reviewGroup(page)).toContainText('耳かき的部分很舒服');
	expect(await requests(page, 'DELETE /me/reviews/12')).toBe(0);
	expect(await requests(page, 'PUT /me/reviews/12')).toBe(1);
});

test('writing and editing a short review counts code points after trimming', async ({ page }, testInfo) => {
	await signIn(page);
	await openWork(page, '示例作品二', 13);
	await reviewGroup(page).getByRole('button', { name: '写短评' }).click();
	await expect(sheet(page)).toBeVisible();
	const text = sheet(page).getByRole('textbox', { name: '短评' });
	await expect(sheet(page).getByText('0 / 2000')).toBeVisible();
	await text.fill('  很好听😀  ');
	await expect(sheet(page).getByText('4 / 2000')).toBeVisible();
	await sheet(page).getByRole('button', { name: '评 5 星' }).click();
	await shot(page, testInfo, 'sheet');
	await sheet(page).getByRole('button', { name: '保存' }).click();
	await expect(sheet(page)).toBeHidden();
	await expect(star(page, 5)).toHaveAttribute('aria-pressed', 'true');
	const edit = reviewGroup(page).getByRole('button', { name: /编辑短评/ });
	await expect(edit).toContainText('很好听😀');

	await edit.click();
	await expect(text).toHaveValue('很好听😀');
	await expect(sheet(page).getByRole('button', { name: '评 5 星' })).toHaveAttribute('aria-pressed', 'true');
	await text.fill('改过的短评');
	// Tapping the chosen star in the sheet clears the draft's rating.
	await sheet(page).getByRole('button', { name: '评 5 星' }).click();
	await sheet(page).getByRole('button', { name: '保存' }).click();
	await expect(sheet(page)).toBeHidden();
	await expect(edit).toContainText('改过的短评');
	await expect(star(page, 5)).toHaveAttribute('aria-pressed', 'false');

	// Cancel throws the draft away.
	await edit.click();
	await text.fill('不会保存');
	await sheet(page).getByRole('button', { name: '取消' }).click();
	await expect(sheet(page)).toBeHidden();
	await expect(edit).toContainText('改过的短评');
});

test('a review longer than 2000 characters cannot be saved', async ({ page }) => {
	await signIn(page);
	await openWork(page, '示例作品二', 13);
	await reviewGroup(page).getByRole('button', { name: '写短评' }).click();
	const text = sheet(page).getByRole('textbox', { name: '短评' });
	const save = sheet(page).getByRole('button', { name: '保存' });
	await text.fill('字'.repeat(2001));
	await expect(sheet(page).getByText('2001 / 2000')).toBeVisible();
	await expect(sheet(page).getByText('短评最多 2000 个字')).toBeVisible();
	await expect(text).toHaveAttribute('aria-invalid', 'true');
	await expect(save).toBeDisabled();

	// White space around the text does not count, as on the server.
	await text.fill(`\n ${'字'.repeat(2000)} \n`);
	await expect(sheet(page).getByText('2000 / 2000')).toBeVisible();
	await expect(save).toBeEnabled();
	await save.click();
	await expect(sheet(page)).toBeHidden();
	await expect(reviewGroup(page).getByRole('button', { name: /编辑短评/ })).toBeVisible();
});

test('deleting a review asks first inside the sheet', async ({ page }) => {
	await signIn(page);
	await openWork(page, '示例作品一', 12);
	await reviewGroup(page).getByRole('button', { name: /编辑短评/ }).click();
	await sheet(page).getByRole('button', { name: '删除评价' }).click();
	await expect(sheet(page).getByText('删除这部作品的评分和短评？')).toBeVisible();
	await sheet(page).getByRole('button', { name: '不删除' }).click();
	await expect(sheet(page).getByText('删除这部作品的评分和短评？')).toBeHidden();
	expect(await requests(page, 'DELETE /me/reviews/12')).toBe(0);

	await sheet(page).getByRole('button', { name: '删除评价' }).click();
	await sheet(page).getByRole('button', { name: '删除', exact: true }).click();
	await expect(sheet(page)).toBeHidden();
	expect(await requests(page, 'DELETE /me/reviews/12')).toBe(1);
	await expect(reviewGroup(page).getByRole('button', { name: '写短评' })).toBeVisible();
	for (const n of [1, 2, 3, 4, 5]) await expect(star(page, n)).toHaveAttribute('aria-pressed', 'false');

	await page.getByRole('button', { name: '返回' }).click();
	await expect(card(page, '示例作品一')).toBeVisible();
	await expect(card(page, '示例作品一').getByRole('img', { name: /我的评分/ })).toHaveCount(0);
});

test('the editor stays closed while a delete is out, then shows what the server kept', async ({ page }) => {
	await signIn(page);
	await openWork(page, '示例作品一', 12);
	const opener = reviewGroup(page).getByRole('button', { name: /写短评|编辑短评/ });
	await opener.click();
	await page.evaluate(() => {
		const hooks = globalThis as { __kikoeruMockFailOnce?: string; __kikoeruMockLatencyMs?: number };
		hooks.__kikoeruMockFailOnce = 'DELETE /me/reviews/12';
		hooks.__kikoeruMockLatencyMs = 2000;
	});
	await sheet(page).getByRole('button', { name: '删除评价' }).click();
	await sheet(page).getByRole('button', { name: '删除', exact: true }).click();
	await sheet(page).getByRole('button', { name: '取消' }).click();
	// The delete shows at once, but the editor cannot open on that guess.
	await expect(opener).toBeDisabled();
	await expect(opener).toBeEnabled({ timeout: 10_000 });
	await page.evaluate(() => {
		(globalThis as { __kikoeruMockLatencyMs?: number }).__kikoeruMockLatencyMs = undefined;
	});
	await opener.click();
	await expect(sheet(page).getByRole('textbox', { name: '短评' })).toHaveValue('耳かき的部分很舒服，后半略长。');
});

test('a failed save keeps the sheet open with the reason', async ({ page }) => {
	await signIn(page);
	await openWork(page, '示例作品一', 12);
	await reviewGroup(page).getByRole('button', { name: /编辑短评/ }).click();
	const text = sheet(page).getByRole('textbox', { name: '短评' });
	await text.fill('这次保存会失败');
	await page.evaluate(() => {
		(globalThis as { __kikoeruMockFailOnce?: string }).__kikoeruMockFailOnce = 'PUT /me/reviews/12';
	});
	await sheet(page).getByRole('button', { name: '保存' }).click();
	await expect(sheet(page).getByRole('alert')).toHaveText('保存失败：模拟的服务器错误');
	await expect(sheet(page)).toBeVisible();
	await expect(text).toHaveValue('这次保存会失败');
	// The page went back to the review the server still has.
	await expect(star(page, 4)).toHaveAttribute('aria-pressed', 'true');

	await sheet(page).getByRole('button', { name: '保存' }).click();
	await expect(sheet(page)).toBeHidden();
	await expect(reviewGroup(page).getByRole('button', { name: /编辑短评/ })).toContainText('这次保存会失败');
});

test('a failed star tap puts the old rating back and says why', async ({ page }) => {
	await signIn(page);
	await openWork(page, '示例作品一', 12);
	await page.evaluate(() => {
		(globalThis as { __kikoeruMockFailOnce?: string }).__kikoeruMockFailOnce = 'PUT /me/reviews/12';
	});
	await star(page, 2).click();
	await expect(reviewGroup(page).getByRole('alert')).toHaveText('评分没有保存：模拟的服务器错误');
	await expect(star(page, 4)).toHaveAttribute('aria-pressed', 'true');
	await expect(star(page, 2)).toHaveAttribute('aria-pressed', 'false');
});

test('我的评价 lists the reviewed works and explains how to rate when empty', async ({ page }, testInfo) => {
	await signIn(page);
	await openReviews(page, testInfo);
	await expect(page.getByRole('article')).toHaveCount(2);
	// Newest change first; the imported review without a time comes last.
	await expect(page.getByRole('article').first()).toContainText('示例作品一');
	await expect(page.getByRole('article').nth(1)).toContainText('已缺失');
	await expect(card(page, '示例作品一').getByRole('img', { name: '我的评分 4 星' })).toBeVisible();
	await expect(card(page, '示例作品四').getByRole('img', { name: /我的评分/ })).toHaveCount(0);
	await shot(page, testInfo, 'list');

	for (const [title, id] of [
		['示例作品一', 12],
		['示例作品四', 15]
	] as const) {
		await openWork(page, title, id);
		await reviewGroup(page).getByRole('button', { name: /编辑短评/ }).click();
		await sheet(page).getByRole('button', { name: '删除评价' }).click();
		await sheet(page).getByRole('button', { name: '删除', exact: true }).click();
		await expect(sheet(page)).toBeHidden();
		await page.getByRole('button', { name: '返回' }).click();
		await expect(page.getByRole('heading', { name: '我的评价', level: 1 })).toBeVisible();
	}
	await expect(page.getByText('还没有评价过的作品')).toBeVisible();
	await expect(page.getByText(/在作品详情页的“我的评价”里点星星打分/)).toBeVisible();
	await shot(page, testInfo, 'empty');
});
