import { describe, expect, it, vi } from 'vitest';
import cleanupRequest from '../../../../contract/m4/cleanup-request.json';
import cleanupResponse from '../../../../contract/m4/cleanup-response.json';
import missing from '../../../../contract/m4/missing.json';
import tagMergeRequest from '../../../../contract/m4/tag-merge-request.json';
import tags from '../../../../contract/m3/tags.json';
import { ApiError } from '../api/errors';
import type { TasksQuery } from '../api/query';
import type { CleanupResponse, MissingReport, Page, TaskSnapshot } from '../api/types';
import { LiveTasks } from './live-tasks.svelte';
import { cleanupSummary, cleanupWarning, MAX_CLEANUP_IDS, MissingStore } from './missing.svelte';
import { mergeProblem, TagMergeStore } from './tag-merge.svelte';

describe('missing cleanup', () => {
	it('sends the selection, reports the result and reloads the list', async () => {
		const empty: MissingReport = { works: [], locations: [] };
		const load = vi.fn().mockResolvedValueOnce(structuredClone(missing)).mockResolvedValueOnce(empty);
		const cleanup = vi.fn(async () => cleanupResponse);
		const store = new MissingStore({ load, cleanup });
		await store.load();
		store.selectAll(true);
		expect(store.selectedCount).toBe(2);
		await store.cleanup();
		expect(cleanup).toHaveBeenCalledWith(cleanupRequest);
		expect(store.result).toBe('已删除 1 部作品、1 个位置。');
		expect(store.selectedCount).toBe(0);
		expect(store.report).toEqual(empty);
	});

	it('keeps the selection and explains a running scan', async () => {
		const cleanup = vi.fn(async (): Promise<CleanupResponse> => {
			throw new ApiError(409, 'scan_running', '有扫描正在进行');
		});
		const store = new MissingStore({ load: async () => structuredClone(missing), cleanup });
		await store.load();
		store.toggleWork(15);
		await store.cleanup();
		expect(store.cleanupError).toContain('扫描结束后点“重试”');
		expect(store.scanBlocked).toBe(true);
		expect(store.selectedWorks).toEqual([15]);
		expect(store.busy).toBe(false);
		// Retrying after the scan sends the confirmed selection again.
		cleanup.mockImplementationOnce(async () => ({ works_deleted: 1, locations_deleted: 0 }));
		await store.retry();
		expect(cleanup).toHaveBeenLastCalledWith({ work_ids: [15], location_ids: [] });
		expect(store.scanBlocked).toBe(false);
	});

	it('offers no retry when the selection changed while the blocked request was pending', async () => {
		let reject!: (e: unknown) => void;
		const cleanup = vi.fn(() => new Promise<CleanupResponse>((_, r) => (reject = r)));
		const store = new MissingStore({ load: async () => structuredClone(missing), cleanup });
		await store.load();
		store.selectAll(true);
		const pending = store.cleanup();
		store.toggleWork(15);
		reject(new ApiError(409, 'scan_running', '有扫描正在进行'));
		await pending;
		expect(store.scanBlocked).toBe(false);
		await store.retry();
		expect(cleanup).toHaveBeenCalledTimes(1);
	});

	it('drops the blocked request when the selection changes, so new rows need confirming', async () => {
		const cleanup = vi.fn(async (): Promise<CleanupResponse> => {
			throw new ApiError(409, 'scan_running', '有扫描正在进行');
		});
		const store = new MissingStore({ load: async () => structuredClone(missing), cleanup });
		await store.load();
		store.toggleLocation(31);
		await store.cleanup();
		expect(store.scanBlocked).toBe(true);
		store.toggleWork(15);
		expect(store.scanBlocked).toBe(false);
		await store.retry();
		expect(cleanup).toHaveBeenCalledTimes(1);
	});

	it('ignores an older load that answers after the cleanup reload', async () => {
		let answerOld!: (r: MissingReport) => void;
		const empty: MissingReport = { works: [], locations: [] };
		const load = vi
			.fn()
			.mockResolvedValueOnce(structuredClone(missing))
			.mockImplementationOnce(() => new Promise<MissingReport>((r) => (answerOld = r)))
			.mockResolvedValueOnce(empty);
		const store = new MissingStore({ load, cleanup: async () => cleanupResponse });
		await store.load();
		store.selectAll(true);
		const refresh = store.load();
		await store.cleanup();
		answerOld(structuredClone(missing));
		await refresh;
		expect(store.report).toEqual(empty);
	});

	it('asks for batches above the server limit instead of sending', async () => {
		const many: MissingReport = {
			works: Array.from({ length: MAX_CLEANUP_IDS + 1 }, (_, i) => ({ id: i + 1, code: `RJ${i}`, title: 't', missing_at: 0 })),
			locations: []
		};
		const cleanup = vi.fn();
		const store = new MissingStore({ load: async () => many, cleanup });
		await store.load();
		store.selectAll(true);
		await store.cleanup();
		expect(cleanup).not.toHaveBeenCalled();
		expect(store.cleanupError).toContain('请分批清理');
	});

	it('does nothing without a selection', async () => {
		const cleanup = vi.fn();
		const store = new MissingStore({ load: async () => structuredClone(missing), cleanup });
		await store.load();
		await store.cleanup();
		expect(cleanup).not.toHaveBeenCalled();
	});

	it('counts rows that came back as skipped', () => {
		expect(cleanupSummary(cleanupRequest, { works_deleted: 0, locations_deleted: 1 })).toBe(
			'已删除 0 部作品、1 个位置。1 项已经重新出现，没有删除。'
		);
	});

	it('warns that deleting a work also deletes favorites and progress', () => {
		expect(cleanupWarning(1, 0)).toContain('1 部作品');
		expect(cleanupWarning(1, 0)).toContain('收藏');
		expect(cleanupWarning(1, 0)).not.toContain('位置');
		expect(cleanupWarning(0, 2)).toContain('2 个位置');
		expect(cleanupWarning(1, 1)).toContain('不能撤销');
	});
});

describe('tag merge', () => {
	it('checks the pair before merging', () => {
		expect(mergeProblem(null, 7, tags)).not.toBeNull();
		expect(mergeProblem(7, 7, tags)).toBe('两个标签相同，不需要合并');
		expect(mergeProblem(9, 99, tags)).toBe('标签已经不存在，请刷新列表');
		expect(mergeProblem(9, 7, tags)).toBeNull();
	});

	it('merges the chosen pair and refreshes the tag list', async () => {
		const listTags = vi
			.fn()
			.mockResolvedValueOnce(structuredClone(tags))
			.mockResolvedValueOnce([{ id: 7, name: 'ASMR', work_count: 128 }]);
		const mergeTags = vi.fn(async () => {});
		const store = new TagMergeStore({ listTags, mergeTags });
		await store.load();
		store.from = 9;
		store.into = 7;
		await store.merge();
		expect(mergeTags).toHaveBeenCalledWith(tagMergeRequest);
		expect(store.result).toBe('已把「耳语」合并进「ASMR」。');
		expect(store.tags).toEqual([{ id: 7, name: 'ASMR', work_count: 128 }]);
		expect(store.from).toBeNull();
		expect(store.into).toBe(7);
	});

	it('ignores an older list that answers after the merge reload', async () => {
		let answerOld!: (t: typeof tags) => void;
		const merged = [{ id: 7, name: 'ASMR', work_count: 128 }];
		const listTags = vi
			.fn()
			.mockResolvedValueOnce(structuredClone(tags))
			.mockImplementationOnce(() => new Promise((r) => (answerOld = r)))
			.mockResolvedValueOnce(merged);
		const store = new TagMergeStore({ listTags, mergeTags: async () => {} });
		await store.load();
		const scanReload = store.load();
		store.from = 9;
		store.into = 7;
		await store.merge();
		answerOld(structuredClone(tags));
		await scanReload;
		expect(store.tags).toEqual(merged);
	});

	it('reloads the list after a failed merge so a stale tag disappears', async () => {
		const listTags = vi
			.fn()
			.mockResolvedValueOnce(structuredClone(tags))
			.mockResolvedValueOnce([{ id: 7, name: 'ASMR', work_count: 120 }]);
		const mergeTags = vi.fn(async () => {
			throw new ApiError(400, 'invalid_request', '没有这个标签');
		});
		const store = new TagMergeStore({ listTags, mergeTags });
		await store.load();
		store.from = 9;
		store.into = 7;
		await store.merge();
		expect(store.mergeError).toBe('合并失败：没有这个标签');
		expect(store.from).toBeNull();
		expect(listTags).toHaveBeenCalledTimes(2);
	});
});

describe('task activity', () => {
	it('counts probes apart from scans and says when a page was full', async () => {
		const { countProbes, TaskActivity } = await import('./task-activity.svelte');
		const page = (items: object[], total = items.length) => ({ items, page: 1, page_size: 100, total }) as never;
		const scan = { id: 1, kind: 'scan', state: 'running', created_at: 1 };
		const probe = { id: 2, kind: 'probe', state: 'running', created_at: 2 };
		expect(countProbes(page([probe]), page([scan, probe], 140))).toEqual({ queued: 1, running: 1, capped: true });

		const listTasks = vi.fn(async ({ state }: { state?: string }) =>
			state === 'running' ? page([scan, probe]) : page([])
		);
		const activity = new TaskActivity({ listTasks });
		await activity.refresh();
		expect(activity.scans.map((t) => t.id)).toEqual([1]);
		expect(activity.probes).toEqual({ queued: 0, running: 1, capped: false });
		expect(activity.busy).toBe(true);
	});

	it('keeps the newest answer when an older poll returns last', async () => {
		const { TaskActivity } = await import('./task-activity.svelte');
		const page = (items: object[]) => ({ items, page: 1, page_size: 100, total: items.length }) as never;
		const scan = { id: 1, kind: 'scan', state: 'running', created_at: 1 };
		let oldRequest = true;
		let answerOld!: () => void;
		const oldAnswer = new Promise<void>((resolve) => (answerOld = resolve));
		const listTasks = vi.fn(async ({ state }: TasksQuery) => {
			const stale = oldRequest;
			if (stale) await oldAnswer;
			return page(stale && state === 'running' ? [scan] : []);
		});
		const activity = new TaskActivity({ listTasks });
		const old = activity.refresh();
		oldRequest = false;
		await activity.refresh();
		answerOld();
		await old;
		expect(activity.scans).toEqual([]);
	});

	it('finds a queued scan behind a full page of probes', async () => {
		const { TaskActivity } = await import('./task-activity.svelte');
		const probe = (id: number) => ({ id, kind: 'probe', state: 'queued', created_at: id });
		const scan = { id: 1, kind: 'scan', state: 'queued', created_at: 1 };
		const queued = [...Array.from({ length: 150 }, (_, i) => probe(200 - i)), scan];
		const listTasks = vi.fn(async ({ state, page = 1, page_size = 100 }: { state?: string; page?: number; page_size?: number }) => {
			const items = state === 'queued' ? queued : [];
			return { items: items.slice((page - 1) * page_size, page * page_size), page, page_size, total: items.length } as never;
		});
		const activity = new TaskActivity({ listTasks });
		await activity.refresh();
		expect(activity.scans.map((t) => t.id)).toEqual([1]);
		expect(activity.probes).toMatchObject({ queued: 100, capped: true });
	});

	it.each(['probe', 'analysis'] as const)('finds and controls queued scans behind 2001 newer %s tasks on a fresh page', async (kind) => {
		const { TaskActivity } = await import('./task-activity.svelte');
		const snapshot = (id: number, kind: TaskSnapshot['kind'], state: TaskSnapshot['state'], queue_position: number | null = null): TaskSnapshot => ({
			id, kind, state, queue_position, created_at: id, updated_at: id,
			progress: null, message: null, result: null
		});
		const tasks = [
			snapshot(1, 'scan', 'running'),
			snapshot(2, 'scan', 'queued', 1),
			snapshot(3, 'scan', 'queued', 2),
			...Array.from({ length: 2001 }, (_, i) => snapshot(i + 4, kind, 'queued'))
		].reverse();
		const listTasks = vi.fn(async ({ state, exclude_kind, page = 1, page_size = 100 }: TasksQuery): Promise<Page<TaskSnapshot>> => {
			const items = tasks.filter((t) => t.state === state && t.kind !== exclude_kind);
			return { items: items.slice((page - 1) * page_size, page * page_size), page, page_size, total: items.length };
		});
		const activity = new TaskActivity({ listTasks });
		await activity.refresh();
		expect(activity.scans.map((t) => t.id)).toEqual([3, 2, 1]);
		expect(activity.probes).toEqual({ queued: kind === 'probe' ? 100 : 0, running: 0, capped: true });

		const cancel = vi.fn(async () => {});
		const moveToFront = vi.fn(async () => {});
		const live = new LiveTasks({
			watch: () => ({ close: () => {}, finished: Promise.resolve(null) }),
			cancel,
			moveToFront
		});
		for (const scan of activity.scans) live.follow(scan.id, `扫描任务 #${scan.id}`, scan);
		expect(live.get(2)?.snapshot?.queue_position).toBe(1);
		expect(live.get(3)?.snapshot?.queue_position).toBe(2);
		await live.moveToFront(3);
		await live.cancel(2);
		expect(moveToFront).toHaveBeenCalledWith(3);
		expect(cancel).toHaveBeenCalledWith(2);
		live.closeAll();
	});
});

