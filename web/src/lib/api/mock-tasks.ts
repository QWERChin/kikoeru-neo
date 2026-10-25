// Mock tasks for the M4 pages: scans that move through their stages on a
// clock, the background probes a scan starts, metadata refreshes, and a
// history of finished tasks so the task list has pages. Loaded only in mock
// mode, through mock.ts. Each scan keeps its own clock, so moving a queued
// scan to the front changes its queue position but not when it starts.
import tasksPage from '../../../../contract/m4/tasks-page.json';
import type { Page, TaskKind, TaskSnapshot, TaskState } from './types';
import { pageParams, paginate } from './mock-catalog';

/** How long each stage of a mock scan lasts. The whole scan takes about 8 seconds. */
export const MOCK_SCAN_TIMING = { queued: 500, discover: 1_000, tracks: 4_000, metadata: 2_000 } as const;
/** Probes one mock scan starts, which run one after another after the track stage begins. */
export const MOCK_PROBES_PER_SCAN = 5;
export const MOCK_PROBE_MS = 2_000;
export const MOCK_REFRESH_MS = 1_500;
/** A cancel takes effect after this long, like a task that stops at a safe point. */
export const MOCK_CANCEL_DELAY_MS = 300;
/** Finished tasks before the samples, so the task list has more than one page. */
export const MOCK_HISTORY_COUNT = 44;
/** Finished loudness analyses among them, which the task list hides by default. */
export const MOCK_ANALYSIS_COUNT = 6;

const METADATA_TOTAL = 12;
const TERMINAL: readonly TaskState[] = ['done', 'failed', 'cancelled', 'interrupted'];
const KINDS: readonly TaskKind[] = ['scan', 'probe', 'transcode', 'metadata', 'analysis'];

interface Timed {
	startedAt: number;
	/** Snapshot fields at `elapsed` milliseconds after the start. */
	at(elapsed: number): Pick<TaskSnapshot, 'state' | 'progress' | 'message'>;
	duration: number;
	cancelAt: number | null;
	onDone?: () => void;
	doneFired: boolean;
}

interface MockTask {
	id: number;
	kind: TaskKind;
	created_at: number;
	/** Started in this mock, as opposed to a sample with a fixed snapshot. */
	timed: Timed | null;
	fixed: TaskSnapshot | null;
	/** The library of a scan, for returning the running task on a repeated request. */
	root?: string;
	/** The work of a metadata refresh. */
	workId?: number;
	/** Queued scans start lowest first. Moving one to the front lowers it. */
	rank: number;
}

const tasks = new Map<number, MockTask>();
/** Clear of the history (1 and up) and of the mock transcode tasks (from 88), see mock.ts. */
const FIRST_STARTED_ID = 10_000;
let nextId = FIRST_STARTED_ID;
// The samples are dated after today, so tasks started now would sort below
// them. Creation times only ever move forward from the newest sample.
let lastCreatedAt = Math.max(...tasksPage.items.map((t) => t.created_at));
function nextCreatedAt(): number {
	lastCreatedAt = Math.max(Date.now(), lastCreatedAt + 1_000);
	return lastCreatedAt;
}

function seed(): void {
	const samples = tasksPage.items as TaskSnapshot[];
	const oldest = Math.min(...samples.map((t) => t.created_at));
	const states: TaskState[] = ['done', 'done', 'done', 'failed', 'interrupted', 'cancelled'];
	const kinds: TaskKind[] = ['probe', 'probe', 'scan', 'metadata', 'transcode'];
	for (let n = 0; n < MOCK_HISTORY_COUNT; n++) {
		const id = 1 + n;
		const kind = kinds[n % kinds.length]!;
		const state = states[n % states.length]!;
		const created_at = oldest - (MOCK_HISTORY_COUNT - n) * 600_000;
		const message =
			state === 'failed' ? '媒体库 云端作品 暂时无法访问' : state === 'interrupted' ? '服务重启时中断' : null;
		tasks.set(id, {
			id,
			kind,
			created_at,
			timed: null,
			rank: id,
			fixed: {
				id,
				kind,
				state,
				progress: state === 'done' ? 1 : null,
				message,
				result: null,
				queue_position: null,
				created_at,
				updated_at: created_at + 60_000
			}
		});
	}
	// IDs right after the history, below the mock transcode tasks (from 88),
	// and created between its rows so the default list leaves gaps in time.
	for (let n = 0; n < MOCK_ANALYSIS_COUNT; n++) {
		const id = 1 + MOCK_HISTORY_COUNT + n;
		const created_at = oldest - (MOCK_HISTORY_COUNT - n * 7) * 600_000 + 300_000;
		tasks.set(id, {
			id,
			kind: 'analysis',
			created_at,
			timed: null,
			rank: id,
			fixed: {
				id,
				kind: 'analysis',
				state: 'done',
				progress: 1,
				message: null,
				result: null,
				queue_position: null,
				created_at,
				updated_at: created_at + 5_000
			}
		});
	}
	for (const sample of samples) {
		tasks.set(sample.id, {
			id: sample.id,
			kind: sample.kind,
			created_at: sample.created_at,
			timed: null,
			rank: sample.id,
			fixed: sample
		});
	}
}
seed();

function snapshot(task: MockTask, now = Date.now()): TaskSnapshot {
	const current = withoutPosition(task, now);
	return current.kind === 'scan' && current.state === 'queued'
		? { ...current, queue_position: queuePosition(task, now) }
		: current;
}

/** Where a queued scan stands among the queued mock scans, like the server's scan queue. */
function queuePosition(task: MockTask, now: number): number {
	const queued = [...tasks.values()]
		.filter((t) => t.kind === 'scan' && withoutPosition(t, now).state === 'queued')
		.sort((a, b) => a.rank - b.rank || a.id - b.id);
	return queued.indexOf(task) + 1;
}

function withoutPosition(task: MockTask, now: number): TaskSnapshot {
	if (task.fixed) return task.fixed;
	const timed = task.timed!;
	const elapsed = now - timed.startedAt;
	const base = { id: task.id, kind: task.kind, result: null, queue_position: null, created_at: task.created_at };
	if (timed.cancelAt !== null && now >= timed.cancelAt) {
		const at = timed.at(timed.cancelAt - timed.startedAt);
		const ended = TERMINAL.includes(at.state);
		return {
			...base,
			...(ended ? at : { state: 'cancelled' as const, progress: at.progress, message: '已取消' }),
			updated_at: task.created_at + (timed.cancelAt - timed.startedAt)
		};
	}
	if (elapsed >= timed.duration && !timed.doneFired) {
		timed.doneFired = true;
		timed.onDone?.();
	}
	return { ...base, ...timed.at(elapsed), updated_at: task.created_at + Math.max(0, Math.min(elapsed, timed.duration)) };
}

function add(kind: TaskKind, timed: Omit<Timed, 'cancelAt' | 'doneFired'>, extra: Partial<MockTask> = {}) {
	const id = nextId++;
	const task: MockTask = {
		id,
		kind,
		created_at: nextCreatedAt(),
		timed: { ...timed, cancelAt: null, doneFired: false },
		fixed: null,
		rank: id,
		...extra
	};
	tasks.set(id, task);
	// Fires the completion even when nobody reads the task in between.
	setTimeout(() => snapshot(task), timed.duration + 1);
	return task;
}

// The track stage counts the works of the one library being scanned, like
// the server's `同步轨道 {done}/{total}`.
const scanTimeline = (worksTotal: number) => (elapsed: number): Pick<TaskSnapshot, 'state' | 'progress' | 'message'> => {
	const { queued, discover, tracks, metadata } = MOCK_SCAN_TIMING;
	if (elapsed < queued) return { state: 'queued', progress: null, message: null };
	let t = elapsed - queued;
	if (t < discover) return { state: 'running', progress: null, message: '发现作品' };
	t -= discover;
	if (t < tracks) {
		const done = Math.floor((t / tracks) * worksTotal);
		return {
			state: 'running',
			progress: round((done / Math.max(1, worksTotal)) * 0.8),
			message: `同步轨道 ${done}/${worksTotal}`
		};
	}
	t -= tracks;
	if (t < metadata) {
		const done = Math.floor((t / metadata) * METADATA_TOTAL);
		return {
			state: 'running',
			progress: round(0.8 + (done / METADATA_TOTAL) * 0.2),
			message: `抓取元数据 ${done}/${METADATA_TOTAL}`
		};
	}
	return { state: 'done', progress: 1, message: null };
};

const round = (x: number) => Math.round(x * 1000) / 1000;

function probeTimeline(wait: number) {
	return (elapsed: number): Pick<TaskSnapshot, 'state' | 'progress' | 'message'> => {
		if (elapsed < wait) return { state: 'queued', progress: null, message: null };
		if (elapsed < wait + MOCK_PROBE_MS) return { state: 'running', progress: null, message: null };
		return { state: 'done', progress: 1, message: null };
	};
}

function isRunning(task: MockTask): boolean {
	return !TERMINAL.includes(snapshot(task).state);
}

/** One scan per library, joining a scan of the same library that is still running. */
export function startScans(
	roots: { id: string; works: number }[],
	onDone: (root: string) => void
): number[] {
	const now = Date.now();
	const { queued, discover, tracks, metadata } = MOCK_SCAN_TIMING;
	return roots.map(({ id: root, works }) => {
		const running = [...tasks.values()].find((t) => t.kind === 'scan' && t.root === root && isRunning(t));
		if (running) return running.id;
		const scan = add('scan', { startedAt: now, duration: queued + discover + tracks + metadata, at: scanTimeline(works), onDone: () => onDone(root) }, { root });
		// Probes are queued when the track stage starts and keep running after
		// the scan ends, which is what the page has to show apart.
		const probesAt = queued + discover;
		setTimeout(() => {
			if (snapshot(scan).state === 'cancelled') return;
			const start = Date.now();
			for (let n = 0; n < MOCK_PROBES_PER_SCAN; n++) {
				add('probe', { startedAt: start, duration: (n + 1) * MOCK_PROBE_MS, at: probeTimeline(n * MOCK_PROBE_MS) });
			}
		}, probesAt);
		return scan.id;
	});
}

/** True while a scan started in this mock is queued or running. */
export function scanRunning(): boolean {
	return [...tasks.values()].some((t) => t.kind === 'scan' && t.timed !== null && isRunning(t));
}

/** True while a scan of this library, alone or as part of "scan all", is queued or running. */
export function scanRunningFor(root: string): boolean {
	return [...tasks.values()].some((t) => t.kind === 'scan' && t.root === root && t.timed !== null && isRunning(t));
}

/** A metadata refresh of one work, joining a running refresh of the same work. */
export function startRefresh(workId: number, onDone: () => void): number {
	const running = [...tasks.values()].find((t) => t.kind === 'metadata' && t.workId === workId && isRunning(t));
	if (running) return running.id;
	const at = (elapsed: number): Pick<TaskSnapshot, 'state' | 'progress' | 'message'> =>
		elapsed < MOCK_REFRESH_MS
			? { state: 'running', progress: null, message: '抓取元数据' }
			: { state: 'done', progress: 1, message: null };
	return add('metadata', { startedAt: Date.now(), duration: MOCK_REFRESH_MS, at, onDone }, { workId }).id;
}

export function getMockTask(id: number): TaskSnapshot | null {
	const task = tasks.get(id);
	return task ? snapshot(task) : null;
}

export type CancelResult = 'ok' | 'not_found' | 'ended';

export function cancelMockTask(id: number): CancelResult {
	const task = tasks.get(id);
	if (!task) return 'not_found';
	const current = snapshot(task);
	if (TERMINAL.includes(current.state)) return 'ended';
	if (task.fixed) {
		// A sample has no clock, so it ends at once.
		task.fixed = { ...task.fixed, state: 'cancelled', message: '已取消', updated_at: Date.now() };
		return 'ok';
	}
	task.timed!.cancelAt ??= Date.now() + MOCK_CANCEL_DELAY_MS;
	return 'ok';
}

export type MoveResult = 'ok' | 'not_found' | 'not_queued_scan';

/** Ranks a queued scan before every other one, as the server's move to the front does. */
export function moveMockTaskToFront(id: number): MoveResult {
	const task = tasks.get(id);
	if (!task) return 'not_found';
	const current = snapshot(task);
	if (current.kind !== 'scan' || current.state !== 'queued') return 'not_queued_scan';
	task.rank = Math.min(...[...tasks.values()].map((t) => t.rank)) - 1;
	return 'ok';
}

/** Null when `state` or `exclude_kind` is not a value the server knows. */
export function listMockTasks(query: Record<string, unknown> | undefined): Page<TaskSnapshot> | null {
	const state = query?.state === undefined ? undefined : String(query.state);
	if (state !== undefined && ![...TERMINAL, 'queued', 'running'].includes(state as TaskState)) return null;
	const excluded = query?.exclude_kind === undefined ? undefined : String(query.exclude_kind);
	if (excluded !== undefined && !KINDS.includes(excluded as TaskKind)) return null;
	const all = [...tasks.values()]
		.map((t) => snapshot(t))
		.filter((s) => state === undefined || s.state === state)
		.filter((s) => s.kind !== excluded)
		.sort((a, b) => b.created_at - a.created_at || b.id - a.id);
	return paginate(all, pageParams(query));
}

export function isTerminalState(state: TaskState): boolean {
	return TERMINAL.includes(state);
}
