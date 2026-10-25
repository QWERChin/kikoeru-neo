<script lang="ts">
	import type { LiveTask } from '../../stores/live-tasks.svelte';
	import { parseScanStage, SCAN_STAGES, TASK_KIND_LABELS, taskStateLabel } from '../../tasks/labels';
	import { canMoveToFront } from '../../tasks/queue-order';
	import { isTerminal } from '../../tasks/task-watch';

	interface Props {
		task: LiveTask;
		onCancel: () => void;
		onDismiss: () => void;
		/** Offered on queued scans that are not next already. */
		onMoveToFront?: () => void;
	}

	let { task, onCancel, onDismiss, onMoveToFront }: Props = $props();

	const MODE_LABELS = {
		connecting: '正在连接',
		stream: '实时更新',
		reconnecting: '连接断开，正在重连',
		polling: '实时连接不可用，每隔几秒刷新一次',
		closed: ''
	} as const;

	let snapshot = $derived(task.snapshot);
	let ended = $derived(snapshot !== null && isTerminal(snapshot.state));
	let stage = $derived(snapshot?.kind === 'scan' && !ended ? parseScanStage(snapshot.message) : null);
	let percent = $derived(snapshot?.progress === null || snapshot?.progress === undefined ? null : Math.round(snapshot.progress * 100));
	let tone = $derived(
		snapshot?.state === 'failed' || snapshot?.state === 'interrupted'
			? 'text-danger'
			: snapshot?.state === 'done'
				? 'text-listening'
				: 'text-secondary'
	);
</script>

<li class="flex flex-col gap-2 border-b border-separator px-4 py-3 last:border-b-0" aria-label={task.label}>
	<div class="flex items-center gap-2">
		<p class="min-w-0 flex-1 truncate font-medium">{task.label}</p>
		<span class="shrink-0 text-[13px] {tone}" aria-live="polite">
			{snapshot ? taskStateLabel(snapshot) : '读取中'}
		</span>
	</div>

	{#if snapshot?.kind === 'scan' && !ended}
		<ol class="flex flex-wrap gap-x-3 gap-y-1 text-[13px]" aria-label="扫描阶段">
			{#each SCAN_STAGES as label, i (label)}
				<li
					class={stage && i < stage.index
						? 'text-secondary line-through decoration-1'
						: stage?.index === i
							? 'font-semibold text-accent-text'
							: 'text-secondary'}
					aria-current={stage?.index === i ? 'step' : undefined}
				>
					{i + 1}. {label}{stage?.index === i && stage.total !== null ? ` ${stage.done}/${stage.total}` : ''}
				</li>
			{/each}
		</ol>
	{/if}

	{#if !ended}
		<div
			class="h-1.5 overflow-hidden rounded-full bg-fill"
			role="progressbar"
			aria-label="{task.label} 的进度"
			aria-valuemin={0}
			aria-valuemax={100}
			aria-valuenow={percent ?? undefined}
		>
			{#if percent !== null}
				<div class="h-full rounded-full bg-accent transition-[width] duration-300" style:width="{percent}%"></div>
			{:else}
				<div class="h-full w-1/3 animate-pulse rounded-full bg-accent/60"></div>
			{/if}
		</div>
	{/if}

	{#if snapshot?.message && !stage}
		<p class="text-[13px] {tone}">{snapshot.message}</p>
	{/if}
	{#if snapshot?.kind === 'scan' && snapshot.state === 'done'}
		<p class="text-[13px] text-secondary">扫描已完成。后台探测另外计算，见下面的“后台探测”。</p>
	{/if}
	{#if task.error}
		<p class="text-[13px] text-danger" role="alert">{task.error}</p>
	{/if}

	<div class="flex items-center gap-3">
		<p class="min-w-0 flex-1 text-[12px] text-secondary">
			{snapshot ? TASK_KIND_LABELS[snapshot.kind] : ''}任务 #{task.id}{!ended && MODE_LABELS[task.mode]
				? ` · ${MODE_LABELS[task.mode]}`
				: ''}
		</p>
		{#if ended}
			<button type="button" class="min-h-11 px-2 text-[15px] text-accent-text" onclick={onDismiss}>移除</button>
		{:else}
			{#if onMoveToFront && canMoveToFront(snapshot)}
				<button
					type="button"
					class="min-h-11 shrink-0 px-2 text-[15px] text-accent-text disabled:opacity-50"
					disabled={task.moving}
					onclick={onMoveToFront}
				>
					{task.moving ? '正在移动' : '移到最前'}
				</button>
			{/if}
			<button
				type="button"
				class="min-h-11 shrink-0 px-2 text-[15px] text-danger disabled:opacity-50"
				disabled={task.cancelling}
				onclick={onCancel}
			>
				{task.cancelling ? '正在取消' : '取消任务'}
			</button>
		{/if}
	</div>
</li>
