import type { LiveTask } from '../stores/live-tasks.svelte';
import { parseScanStage } from './labels';
import { isTerminal } from './task-watch';

/** What the settings page remembers about a scan it started. */
export interface StartedScan {
	label: string;
	/** The library asked for, or null for "scan all". */
	root: string | null;
}

/** A followed scan that may belong to a library. */
export interface RootScan {
	task: LiveTask;
	/** The page knows this task scans the library, rather than one of several. */
	exact: boolean;
}

/**
 * The followed scans that may be scanning one library and have not ended.
 * A task snapshot does not name its library, so this relies on what the
 * page asked for. "Scan all" covers every library but answers one task per
 * library in no fixed order, so its tasks are only exact when there is a
 * single library. A scan the activity poll found without the page starting
 * it is placed only in that case too.
 */
export function scansOfRoot(
	rootId: string,
	rootCount: number,
	items: readonly LiveTask[],
	started: ReadonlyMap<number, StartedScan>
): RootScan[] {
	const scans: RootScan[] = [];
	for (const task of items) {
		if (task.snapshot && (task.snapshot.kind !== 'scan' || isTerminal(task.snapshot.state))) continue;
		const root = started.get(task.id)?.root;
		if (root === rootId) scans.push({ task, exact: true });
		else if (rootCount === 1) scans.push({ task, exact: true });
		else if (root === null) scans.push({ task, exact: false });
	}
	return scans;
}

/**
 * The line a library row shows in place of its last scan time while one of
 * `scans` is queued or running, or null when there is none. The stage is
 * shown only for a single scan known to be this library's. Otherwise the
 * scan of another library could be shown, even after this one finished.
 */
export function rootScanText(scans: readonly RootScan[]): string | null {
	if (scans.length === 0) return null;
	const running = scans.find((s) => s.task.snapshot?.state === 'running');
	if (!running) return '排队等待扫描';
	if (scans.length > 1 || !running.exact) return '正在扫描';
	const stage = parseScanStage(running.task.snapshot!.message);
	if (!stage) return '正在扫描';
	const count = stage.done !== null && stage.total !== null ? ` ${stage.done}/${stage.total}` : '';
	return `正在扫描 · ${stage.label}${count}`;
}
