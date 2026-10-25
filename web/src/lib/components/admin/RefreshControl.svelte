<script lang="ts">
	import { onDestroy, untrack } from 'svelte';
	import { ApiError, cancelTask, refreshWork, taskWatchDeps } from '../../api/index';
	import { LiveTasks } from '../../stores/live-tasks.svelte';
	import { metadataSaves } from '../../stores/metadata-saves.svelte';
	import { TASK_STATE_LABELS } from '../../tasks/labels';
	import { isTerminal, watchTask } from '../../tasks/task-watch';
	import ConfirmDialog from '../ConfirmDialog.svelte';

	interface Props {
		workId: number;
		/** Called when the refresh task is done, so the page reloads the work. */
		onRefreshed: () => void;
	}

	let { workId, onRefreshed }: Props = $props();
	const uid = $props.id();

	let force = $state(false);
	let confirming = $state(false);
	let starting = $state(false);
	let taskId = $state<number | null>(null);
	let startError = $state<string | null>(null);

	// LiveTasks stops following once the component is gone, including a task
	// whose id only arrives after that.
	const live = new LiveTasks(
		{ watch: (id, options) => watchTask(id, options, taskWatchDeps), cancel: cancelTask },
		(last) => {
			metadataSaves.refreshEnded(workId, last.id);
			if (last.id === taskId && last.state === 'done') onRefreshed();
		}
	);

	// A refresh started on an earlier visit is followed again, so its end
	// still unlocks editing and reloads the work.
	// The lookup stays tracked, because a refresh sent on an earlier visit
	// may only learn its task ID after this page opened.
	$effect(() => {
		const existing = metadataSaves.refreshTask(workId);
		if (existing === null) return;
		untrack(() => {
			if (taskId === existing) return;
			taskId = existing;
			live.follow(existing, '重新抓取');
		});
	});

	let followed = $derived(taskId === null ? undefined : live.get(taskId));
	let snapshot = $derived(followed?.snapshot ?? null);
	let error = $derived(startError ?? (followed?.error ? `无法读取抓取进度：${followed.error}` : null));
	// A refresh sent while a metadata save is in flight could finish first and
	// then be undone by the older save, so the two never overlap.
	let saving = $derived(metadataSaves.isSaving(workId));
	// The shared state also covers a refresh sent on an earlier visit whose
	// answer has not arrived yet, so a second one cannot start meanwhile.
	let active = $derived(
		starting ||
			metadataSaves.isRefreshing(workId) ||
			// A watch that ended without a terminal snapshot failed, so it no
			// longer blocks a retry.
			(followed !== undefined && followed.mode !== 'closed' && !(snapshot && isTerminal(snapshot.state)))
	);

	// A task that cannot be followed (for example it is gone) must not keep
	// the editor locked.
	$effect(() => {
		if (taskId !== null && followed?.error) metadataSaves.refreshEnded(workId, taskId);
	});

	// Leaving the page stops following the task. The refresh itself goes on.
	onDestroy(() => live.closeAll());

	function request() {
		if (saving || active) return;
		if (force) confirming = true;
		else void start(false);
	}

	async function start(withForce: boolean) {
		starting = true;
		metadataSaves.markRefreshStarting(workId, true);
		startError = null;
		if (taskId !== null) live.dismiss(taskId);
		taskId = null;
		try {
			const { task_id } = await refreshWork(workId, withForce);
			taskId = task_id;
			metadataSaves.refreshStarted(workId, task_id);
			live.follow(task_id, '重新抓取');
		} catch (e) {
			startError = `重新抓取没有开始：${e instanceof ApiError ? e.message : '未知错误'}`;
		} finally {
			starting = false;
			metadataSaves.markRefreshStarting(workId, false);
		}
	}
</script>

<!-- One list like the settings page's: the action row, then its option
     with the explanation under it. Progress and errors go below the list. -->
<div class="overflow-hidden rounded-xl bg-surface">
	<button
		type="button"
		class="flex min-h-11 w-full items-center border-b border-separator px-4 py-3 text-left text-accent-text hover:bg-fill disabled:opacity-50"
		disabled={active || saving}
		onclick={request}
	>
		{active ? '正在抓取' : '重新抓取'}
	</button>
	<label class="flex min-h-11 items-start gap-3 px-4 py-3">
		<input
			type="checkbox"
			class="mt-0.5 size-5 shrink-0 accent-[var(--accent)]"
			aria-label="强制覆盖手动修改"
			aria-describedby="{uid}-hint"
			bind:checked={force}
			disabled={active}
		/>
		<span class="min-w-0 flex-1">
			<span class="block">强制覆盖手动修改</span>
			<span
				id="{uid}-hint"
				class="block text-[13px] {force ? 'text-danger' : 'text-secondary'}"
				role={force ? 'note' : undefined}
			>
				{#if force}
					强制重新抓取会用 DLsite 的数据覆盖手动修改过的元数据，以及导入或手动换过的封面，覆盖后不能恢复。
				{:else}
					普通的重新抓取不会改动手动修改过的元数据，也不会替换导入或手动换过的封面。
				{/if}
			</span>
		</span>
	</label>
</div>
{#if saving}
	<p class="px-1 pt-2 text-[13px] text-secondary">正在保存元数据，保存完成后才能重新抓取。</p>
{/if}
{#if snapshot}
	<p
		class="px-1 pt-2 text-[13px] {snapshot.state === 'failed' || snapshot.state === 'interrupted'
			? 'text-danger'
			: 'text-secondary'}"
		role="status"
	>
		重新抓取：{TASK_STATE_LABELS[snapshot.state]}{snapshot.message ? ` · ${snapshot.message}` : ''}
	</p>
{/if}
{#if error}
	<p class="px-1 pt-2 text-[13px] text-danger" role="alert">{error}</p>
{/if}

<ConfirmDialog
	bind:open={confirming}
	title="强制重新抓取？"
	confirmLabel="覆盖并抓取"
	danger
	onConfirm={() => void start(true)}
>
	<p>DLsite 的数据会覆盖这部作品手动修改过的标题、社团、声优、标签和发售日，以及导入或手动换过的封面。覆盖之后不能恢复。</p>
</ConfirmDialog>
