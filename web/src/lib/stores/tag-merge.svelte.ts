import { ApiError } from '../api/errors';
import type { TagCount, TagMergeRequest } from '../api/types';

export interface TagMergeDeps {
	listTags(): Promise<TagCount[]>;
	mergeTags(body: TagMergeRequest): Promise<void>;
}

/** Why the chosen pair cannot be merged, or null when it can. */
export function mergeProblem(from: number | null, into: number | null, tags: TagCount[] | null): string | null {
	if (from === null || into === null) return '先选要合并的标签和目标标签';
	if (from === into) return '两个标签相同，不需要合并';
	if (!tags?.some((t) => t.id === from) || !tags.some((t) => t.id === into)) return '标签已经不存在，请刷新列表';
	return null;
}

/** The tag merge form on the settings page: pick a source and a target, merge, reload. */
export class TagMergeStore {
	tags = $state<TagCount[] | null>(null);
	loadError = $state<string | null>(null);
	from = $state<number | null>(null);
	into = $state<number | null>(null);
	busy = $state(false);
	result = $state<string | null>(null);
	mergeError = $state<string | null>(null);
	#deps: TagMergeDeps;
	// Only the newest load may set the list, so a reload that a scan started
	// cannot bring back a tag that a merge has just deleted.
	#generation = 0;

	constructor(deps: TagMergeDeps) {
		this.#deps = deps;
	}

	get problem(): string | null {
		return mergeProblem(this.from, this.into, this.tags);
	}

	nameOf(id: number | null): string {
		return this.tags?.find((t) => t.id === id)?.name ?? '';
	}

	async load(): Promise<void> {
		const generation = ++this.#generation;
		this.loadError = null;
		try {
			const tags = await this.#deps.listTags();
			if (generation !== this.#generation) return;
			this.tags = tags;
			if (!tags.some((t) => t.id === this.from)) this.from = null;
			if (!tags.some((t) => t.id === this.into)) this.into = null;
		} catch (e) {
			if (generation !== this.#generation) return;
			this.loadError = e instanceof ApiError ? e.message : '加载失败';
		}
	}

	/** Merges the chosen pair. The page asks for confirmation before calling this. */
	async merge(): Promise<void> {
		if (this.busy || this.problem !== null || this.from === null || this.into === null) return;
		const request = { from: this.from, into: this.into };
		const fromName = this.nameOf(request.from);
		const intoName = this.nameOf(request.into);
		this.busy = true;
		this.result = null;
		this.mergeError = null;
		try {
			await this.#deps.mergeTags(request);
			this.result = `已把「${fromName}」合并进「${intoName}」。`;
			this.from = null;
			await this.load();
		} catch (e) {
			this.mergeError = `合并失败：${e instanceof ApiError ? e.message : '未知错误'}`;
			// The list may be out of date, for example when a tag was merged elsewhere.
			await this.load();
		} finally {
			this.busy = false;
		}
	}
}
