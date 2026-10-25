import type { TaskKind, TaskSnapshot, TaskState } from '../api/types';

export const TASK_KIND_LABELS: Record<TaskKind, string> = {
	scan: '扫描',
	probe: '后台探测',
	transcode: '转码',
	metadata: '抓取元数据',
	analysis: '响度分析'
};

export const TASK_STATE_LABELS: Record<TaskState, string> = {
	queued: '排队中',
	running: '进行中',
	done: '已完成',
	failed: '失败',
	cancelled: '已取消',
	interrupted: '已中断'
};

export const TASK_STATES = Object.keys(TASK_STATE_LABELS) as TaskState[];

/** The state, with a queued scan's place in the queue, such as `排队中 · 第 2 位`. */
export function taskStateLabel(snapshot: Pick<TaskSnapshot, 'state' | 'queue_position'>): string {
	const label = TASK_STATE_LABELS[snapshot.state];
	return snapshot.state === 'queued' && snapshot.queue_position !== null
		? `${label} · 第 ${snapshot.queue_position} 位`
		: label;
}

/** The stages a scan task reports in its message (book/src/scanning.md). */
export const SCAN_STAGES = ['发现作品', '同步轨道', '抓取元数据', '读取封面'] as const;

export interface ScanStage {
	/** Index into SCAN_STAGES. */
	index: number;
	label: (typeof SCAN_STAGES)[number];
	done: number | null;
	total: number | null;
}

/**
 * Reads the stage out of a scan message such as `同步轨道 410/1200`. The
 * message is text for people, so an unknown one gives null and the page
 * shows it as it is.
 */
export function parseScanStage(message: string | null): ScanStage | null {
	if (message === null) return null;
	const match = /^(\S+)(?:\s+(\d+)\/(\d+))?$/.exec(message.trim());
	if (!match) return null;
	const label = match[1] as ScanStage['label'];
	const index = SCAN_STAGES.indexOf(label);
	if (index < 0) return null;
	return {
		index,
		label,
		done: match[2] === undefined ? null : Number(match[2]),
		total: match[3] === undefined ? null : Number(match[3])
	};
}

/** The last page number of a list with `total` rows, at least 1. */
export function lastPage(total: number, pageSize: number): number {
	return Math.max(1, Math.ceil(total / pageSize));
}

/**
 * Tasks that ended can leave a later page of a filtered list empty. Returns
 * the last page that still has rows when that happened, otherwise null.
 */
export function pageAfterShrink(rows: number, total: number, requested: number, pageSize: number): number | null {
	const last = lastPage(total, pageSize);
	return rows === 0 && requested > last ? last : null;
}
