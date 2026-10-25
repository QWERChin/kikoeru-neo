import { ApiError } from '../api/errors';
import type { CleanupRequest, CleanupResponse, MissingReport } from '../api/types';

export interface MissingDeps {
	load(): Promise<MissingReport>;
	cleanup(body: CleanupRequest): Promise<CleanupResponse>;
}

/** The server takes at most this many IDs in one cleanup request. */
export const MAX_CLEANUP_IDS = 1000;

const toError = (e: unknown, fallback: string) => (e instanceof ApiError ? e : new ApiError(0, 'unknown', fallback));

/** What the confirm dialog says before anything is deleted. */
export function cleanupWarning(works: number, locations: number): string {
	const parts = [works > 0 && `${works} 部作品`, locations > 0 && `${locations} 个位置`].filter(Boolean);
	const lines = [`将永久删除${parts.join('和')}。`];
	if (works > 0) lines.push('删除作品会一并删除它的轨道、收藏和所有人的播放进度。');
	if (locations > 0) lines.push('删除位置会删除这个位置的轨道，指向这些轨道的播放进度会保留，但不再对应轨道。');
	lines.push('这个操作不能撤销。期间重新出现的条目会被跳过。');
	return lines.join('');
}

/** The result line after a cleanup, naming rows that came back and were kept. */
export function cleanupSummary(request: CleanupRequest, response: CleanupResponse): string {
	const text = `已删除 ${response.works_deleted} 部作品、${response.locations_deleted} 个位置。`;
	const skipped =
		request.work_ids.length - response.works_deleted + (request.location_ids.length - response.locations_deleted);
	return skipped > 0 ? `${text}${skipped} 项已经重新出现，没有删除。` : text;
}

/** The settings page's list of missing works and locations, with the selection to clean up. */
export class MissingStore {
	report = $state<MissingReport | null>(null);
	error = $state<ApiError | null>(null);
	selectedWorks = $state<number[]>([]);
	selectedLocations = $state<number[]>([]);
	busy = $state(false);
	result = $state<string | null>(null);
	cleanupError = $state<string | null>(null);
	/** The last cleanup hit a running scan, so the page offers to try again. */
	scanBlocked = $state(false);
	#deps: MissingDeps;
	// Only the newest load may set the report, so a refresh that answers
	// after a cleanup's reload cannot bring deleted rows back.
	#generation = 0;
	#blocked: CleanupRequest | null = null;
	// Bumped by every change of the selection. A request that a scan blocked
	// can only be retried while the selection is still the one confirmed.
	#selectionRevision = 0;

	constructor(deps: MissingDeps) {
		this.#deps = deps;
	}

	get selectedCount(): number {
		return this.selectedWorks.length + this.selectedLocations.length;
	}

	async load(): Promise<void> {
		const generation = ++this.#generation;
		this.error = null;
		try {
			const report = await this.#deps.load();
			if (generation !== this.#generation) return;
			this.report = report;
			// Rows that are gone cannot stay selected.
			this.selectedWorks = this.selectedWorks.filter((id) => report.works.some((w) => w.id === id));
			this.selectedLocations = this.selectedLocations.filter((id) => report.locations.some((l) => l.id === id));
		} catch (e) {
			if (generation === this.#generation) this.error = toError(e, '加载失败');
		}
	}

	#selectionChanged(): void {
		this.#selectionRevision++;
		this.#setBlocked(null);
	}

	#setBlocked(request: CleanupRequest | null): void {
		this.#blocked = request;
		this.scanBlocked = request !== null;
	}

	toggleWork(id: number): void {
		this.#selectionChanged();
		this.selectedWorks = toggle(this.selectedWorks, id);
	}

	toggleLocation(id: number): void {
		this.#selectionChanged();
		this.selectedLocations = toggle(this.selectedLocations, id);
	}

	selectAll(selected: boolean): void {
		this.#selectionChanged();
		this.selectedWorks = selected ? (this.report?.works.map((w) => w.id) ?? []) : [];
		this.selectedLocations = selected ? (this.report?.locations.map((l) => l.id) ?? []) : [];
	}

	/** Deletes the selection. The page asks for confirmation before calling this. */
	cleanup(): Promise<void> {
		if (this.busy || this.selectedCount === 0) return Promise.resolve();
		if (this.selectedCount > MAX_CLEANUP_IDS) {
			this.result = null;
			this.cleanupError = `一次最多清理 ${MAX_CLEANUP_IDS} 项，现在选了 ${this.selectedCount} 项，请分批清理。`;
			return Promise.resolve();
		}
		return this.#send({ work_ids: [...this.selectedWorks], location_ids: [...this.selectedLocations] });
	}

	/**
	 * Sends the request that a running scan blocked, exactly as it was
	 * confirmed. Changing the selection drops it, so new rows always go
	 * through the confirm dialog.
	 */
	retry(): Promise<void> {
		const request = this.#blocked;
		if (this.busy || !request) return Promise.resolve();
		return this.#send(request);
	}

	async #send(request: CleanupRequest): Promise<void> {
		const revision = this.#selectionRevision;
		this.busy = true;
		this.result = null;
		this.cleanupError = null;
		this.#setBlocked(null);
		try {
			const response = await this.#deps.cleanup(request);
			this.result = cleanupSummary(request, response);
			this.selectedWorks = [];
			this.selectedLocations = [];
			await this.load();
		} catch (e) {
			const error = toError(e, '清理失败');
			const unchanged = revision === this.#selectionRevision;
			this.#setBlocked(error.code === 'scan_running' && unchanged ? request : null);
			this.cleanupError =
				error.code === 'scan_running' ? '有扫描正在进行，现在不能清理。选中的条目都还在，扫描结束后点“重试”。改了勾选就要重新确认。' : `清理失败：${error.message}`;
		} finally {
			this.busy = false;
		}
	}
}

function toggle(list: number[], id: number): number[] {
	return list.includes(id) ? list.filter((x) => x !== id) : [...list, id];
}
