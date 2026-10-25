import { describe, expect, it } from 'vitest';
import type { TaskSnapshot } from '../api/types';
import type { LiveTask } from '../stores/live-tasks.svelte';
import { rootScanText, scansOfRoot, type RootScan, type StartedScan } from './root-scan';

function task(id: number, snapshot: Partial<TaskSnapshot> | null): LiveTask {
	return {
		id,
		label: `扫描任务 #${id}`,
		snapshot: snapshot && {
			id,
			kind: 'scan',
			state: 'running',
			progress: null,
			message: null,
			result: null,
			queue_position: null,
			created_at: 0,
			updated_at: 0,
			...snapshot
		},
		mode: 'stream',
		error: null,
		cancelling: false,
		moving: false
	};
}

describe('scansOfRoot', () => {
	const started = new Map<number, StartedScan>([
		[1, { label: 'A', root: 'a' }],
		[2, { label: 'B', root: 'b' }],
		[3, { label: '全部媒体库', root: null }]
	]);

	it('matches the library a scan was started for', () => {
		const items = [task(1, { state: 'running' }), task(2, { state: 'queued' })];
		expect(scansOfRoot('a', 2, items, started).map((s) => s.task.id)).toEqual([1]);
		expect(scansOfRoot('b', 2, items, started).map((s) => s.task.id)).toEqual([2]);
	});

	it('counts a scan whose first snapshot has not arrived yet', () => {
		expect(scansOfRoot('a', 2, [task(1, null)], started)).toHaveLength(1);
	});

	it('gives a scan of every library to each of them, without claiming it', () => {
		const items = [task(3, { state: 'running' })];
		expect(scansOfRoot('a', 2, items, started)).toEqual([{ task: items[0], exact: false }]);
		expect(scansOfRoot('b', 2, items, started)).toEqual([{ task: items[0], exact: false }]);
		expect(scansOfRoot('a', 1, items, started)).toEqual([{ task: items[0], exact: true }]);
	});

	it('keeps the last running task of a scan of every library generic', () => {
		// Two tasks from one "scan all": one finished, the other may be either library's.
		const all = new Map<number, StartedScan>([
			[4, { label: '全部媒体库 1/2', root: null }],
			[5, { label: '全部媒体库 2/2', root: null }]
		]);
		const items = [task(4, { state: 'done' }), task(5, { message: '同步轨道 3/9' })];
		expect(rootScanText(scansOfRoot('a', 2, items, all))).toBe('正在扫描');
		expect(rootScanText(scansOfRoot('b', 2, items, all))).toBe('正在扫描');
	});

	it('drops scans that ended', () => {
		const items = [task(1, { state: 'done' }), task(2, { state: 'cancelled' })];
		expect(scansOfRoot('a', 2, items, started)).toEqual([]);
		expect(scansOfRoot('b', 2, items, started)).toEqual([]);
	});

	it('places a scan found by the poll only when there is one library', () => {
		const items = [task(9, { state: 'running' })];
		expect(scansOfRoot('a', 1, items, started)).toHaveLength(1);
		expect(scansOfRoot('a', 2, items, started)).toEqual([]);
	});
});

const exact = (t: LiveTask): RootScan => ({ task: t, exact: true });

describe('rootScanText', () => {
	it('is null without a scan, so the last scan time shows', () => {
		expect(rootScanText([])).toBeNull();
	});

	it('says a queued or just-started scan is waiting', () => {
		expect(rootScanText([exact(task(1, { state: 'queued' }))])).toBe('排队等待扫描');
		expect(rootScanText([exact(task(1, null))])).toBe('排队等待扫描');
	});

	it('shows the stage and count of a running scan', () => {
		expect(rootScanText([exact(task(1, { message: '同步轨道 410/1200' }))])).toBe('正在扫描 · 同步轨道 410/1200');
		expect(rootScanText([exact(task(1, { message: '发现作品' }))])).toBe('正在扫描 · 发现作品');
		expect(rootScanText([exact(task(1, { message: 'something else' }))])).toBe('正在扫描');
	});

	it('gives no stage when it cannot tell which of several scans is this library', () => {
		const scans = [exact(task(1, { message: '同步轨道 1/9' })), exact(task(2, { state: 'queued' }))];
		expect(rootScanText(scans)).toBe('正在扫描');
	});
});
