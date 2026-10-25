import { describe, expect, it } from 'vitest';
import tasksPage from '../../../../contract/m4/tasks-page.json';
import { lastPage, pageAfterShrink, parseScanStage, taskStateLabel } from './labels';

describe('parseScanStage', () => {
	it('reads the stage and counts of the sample scan', () => {
		expect(parseScanStage(tasksPage.items[0]?.message ?? null)).toEqual({
			index: 1,
			label: '同步轨道',
			done: 410,
			total: 1200
		});
	});

	it('reads a stage without counts', () => {
		expect(parseScanStage('发现作品')).toMatchObject({ index: 0, done: null, total: null });
		expect(parseScanStage('抓取元数据 3/9')).toMatchObject({ index: 2, done: 3, total: 9 });
		expect(parseScanStage('读取封面 1/4')).toMatchObject({ index: 3, done: 1, total: 4 });
	});

	it('gives null for messages that are not a known stage', () => {
		expect(parseScanStage(null)).toBeNull();
		expect(parseScanStage('媒体库 本地作品 暂时无法访问')).toBeNull();
		expect(parseScanStage('同步轨道 很多')).toBeNull();
	});
});

describe('lastPage', () => {
	it('is at least 1 and rounds up', () => {
		expect(lastPage(0, 20)).toBe(1);
		expect(lastPage(20, 20)).toBe(1);
		expect(lastPage(21, 20)).toBe(2);
	});
});

describe('pageAfterShrink', () => {
	it('moves an empty later page back to the last page with rows', () => {
		// Page 3 of 41 queued tasks, after 25 of them ended.
		expect(pageAfterShrink(0, 16, 3, 20)).toBe(1);
		expect(pageAfterShrink(0, 0, 2, 20)).toBe(1);
	});

	it('keeps a page that has rows, and the first page even when empty', () => {
		expect(pageAfterShrink(5, 45, 3, 20)).toBeNull();
		expect(pageAfterShrink(0, 0, 1, 20)).toBeNull();
	});
});

describe('taskStateLabel', () => {
	it('adds the place in the queue to a queued scan', () => {
		expect(taskStateLabel({ state: 'queued', queue_position: 2 })).toBe('排队中 · 第 2 位');
		expect(taskStateLabel({ state: 'queued', queue_position: null })).toBe('排队中');
		expect(taskStateLabel({ state: 'running', queue_position: null })).toBe('进行中');
	});
});
