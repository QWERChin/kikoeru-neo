<script lang="ts">
	import { ApiError, cancelTask, listTasks } from '../../api/index';
	import type { TasksQuery } from '../../api/query';
	import type { Page, TaskSnapshot, TaskState } from '../../api/types';
	import { lastPage, pageAfterShrink, TASK_KIND_LABELS, TASK_STATE_LABELS, TASK_STATES, taskStateLabel } from '../../tasks/labels';
	import { isTerminal } from '../../tasks/task-watch';

	interface Props {
		/** Bumped by the page when a followed task ends, so the list reloads. */
		version: number;
	}

	let { version }: Props = $props();

	const PAGE_SIZE = 20;
	const dateFormat = new Intl.DateTimeFormat('zh-CN', { dateStyle: 'short', timeStyle: 'medium' });

	let pageNumber = $state(1);
	let stateFilter = $state<TaskState | ''>('');
	// Every first play queues a loudness analysis, so they would bury scans
	// and transcodes. The server log still has a line for each one.
	let showAnalysis = $state(false);
	let data = $state<Page<TaskSnapshot> | null>(null);
	let error = $state<string | null>(null);
	let loading = $state(false);
	let attempt = $state(0);
	let cancelError = $state<string | null>(null);
	let cancelling = $state<number[]>([]);

	let pageCount = $derived(data ? lastPage(data.total, PAGE_SIZE) : 1);

	$effect(() => {
		const requested = pageNumber;
		const query: TasksQuery = {
			page: requested,
			page_size: PAGE_SIZE,
			state: stateFilter || undefined,
			exclude_kind: showAnalysis ? undefined : 'analysis'
		};
		void version;
		void attempt;
		let stale = false;
		loading = true;
		error = null;
		listTasks(query)
			.then((page) => {
				if (stale) return;
				const jump = pageAfterShrink(page.items.length, page.total, requested, PAGE_SIZE);
				if (jump !== null) {
					pageNumber = jump;
					return;
				}
				data = page;
			})
			.catch((e: unknown) => {
				if (!stale) error = e instanceof ApiError ? e.message : '加载失败';
			})
			.finally(() => {
				if (!stale) loading = false;
			});
		return () => {
			stale = true;
		};
	});

	function changeFilter(value: string) {
		stateFilter = value as TaskState | '';
		pageNumber = 1;
	}

	function toggleAnalysis(checked: boolean) {
		showAnalysis = checked;
		pageNumber = 1;
	}

	async function cancel(id: number) {
		cancelError = null;
		cancelling = [...cancelling, id];
		try {
			await cancelTask(id);
		} catch (e) {
			cancelError = `取消任务 #${id} 失败：${e instanceof ApiError ? e.message : '未知错误'}`;
		} finally {
			cancelling = cancelling.filter((x) => x !== id);
			attempt++;
		}
	}
</script>

<section aria-labelledby="settings-tasks">
	<div class="flex flex-wrap items-center gap-2 px-1 pb-1">
		<h2 id="settings-tasks" class="flex-1 text-[13px] font-semibold tracking-wide text-secondary">任务</h2>
		<label class="flex min-h-11 items-center gap-2 text-[13px] text-secondary">
			<input
				type="checkbox"
				class="size-5 shrink-0 accent-[var(--accent)]"
				checked={showAnalysis}
				onchange={(e) => toggleAnalysis(e.currentTarget.checked)}
			/>
			<span>显示响度分析</span>
		</label>
		<label class="flex items-center gap-2 text-[13px] text-secondary">
			<span aria-hidden="true">状态</span>
			<select
				aria-label="按状态筛选"
				class="min-h-11 rounded-lg bg-surface px-2 text-[15px] text-text"
				value={stateFilter}
				onchange={(e) => changeFilter(e.currentTarget.value)}
			>
				<option value="">全部</option>
				{#each TASK_STATES as state (state)}
					<option value={state}>{TASK_STATE_LABELS[state]}</option>
				{/each}
			</select>
		</label>
		<button type="button" class="min-h-11 px-2 text-[15px] text-accent-text" onclick={() => attempt++}>刷新</button>
	</div>
	{#if cancelError}
		<p class="mb-2 rounded-xl bg-surface px-4 py-3 text-[15px] text-danger" role="alert">{cancelError}</p>
	{/if}
	{#if error}
		<p class="rounded-xl bg-surface px-4 py-3 text-[15px] text-danger" role="alert">任务列表加载失败：{error}</p>
	{:else if !data}
		<div class="h-24 animate-pulse rounded-xl bg-surface" aria-busy="true" aria-label="正在加载"></div>
	{:else if data.items.length === 0}
		<p class="rounded-xl bg-surface px-4 py-3 text-[15px] text-secondary">
			{stateFilter ? `没有${TASK_STATE_LABELS[stateFilter]}的任务。` : '还没有任务。'}{showAnalysis
				? ''
				: '自动的响度分析没有列出。'}
		</p>
	{:else}
		<ul class="overflow-hidden rounded-xl bg-surface" aria-label="任务列表" aria-busy={loading}>
			{#each data.items as task (task.id)}
				<li class="flex min-h-11 items-center gap-3 border-b border-separator px-4 py-2.5 last:border-b-0">
					<div class="min-w-0 flex-1">
						<p class="truncate">
							{TASK_KIND_LABELS[task.kind]}
							<span class="text-[13px] text-secondary">#{task.id}</span>
						</p>
						<p class="truncate text-[13px] text-secondary">
							{dateFormat.format(task.created_at)}{task.message ? ` · ${task.message}` : ''}
						</p>
					</div>
					<span
						class="shrink-0 text-[13px] {task.state === 'failed' || task.state === 'interrupted'
							? 'text-danger'
							: task.state === 'running'
								? 'text-accent-text'
								: 'text-secondary'}"
					>
						{taskStateLabel(task)}{task.state === 'running' && task.progress !== null
							? ` ${Math.round(task.progress * 100)}%`
							: ''}
					</span>
					{#if !isTerminal(task.state)}
						<button
							type="button"
							class="min-h-11 shrink-0 px-1 text-[15px] text-danger disabled:opacity-50"
							aria-label="取消任务 #{task.id}"
							disabled={cancelling.includes(task.id)}
							onclick={() => void cancel(task.id)}
						>
							取消
						</button>
					{/if}
				</li>
			{/each}
		</ul>
		<nav class="flex items-center justify-between px-1 pt-1" aria-label="任务翻页">
			<button
				type="button"
				class="min-h-11 px-2 text-[15px] text-accent-text disabled:text-finished"
				disabled={pageNumber <= 1}
				onclick={() => pageNumber--}
			>
				上一页
			</button>
			<span class="text-[13px] text-secondary">第 {pageNumber} / {pageCount} 页 · 共 {data.total} 个</span>
			<button
				type="button"
				class="min-h-11 px-2 text-[15px] text-accent-text disabled:text-finished"
				disabled={pageNumber >= pageCount}
				onclick={() => pageNumber++}
			>
				下一页
			</button>
		</nav>
	{/if}
</section>
