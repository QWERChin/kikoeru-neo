<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { ApiError, DEFAULT_PAGE_SIZE, getWork, listHistory } from '#lib/api/index.ts';
	import type { WorkSummary } from '#lib/api/types/index.ts';
	import Icon from '#lib/components/Icon.svelte';
	import LargeTitle from '#lib/components/LargeTitle.svelte';
	import LoadError from '#lib/components/LoadError.svelte';
	import PagedWorks from '#lib/components/PagedWorks.svelte';
	import { formatClock } from '#lib/format.ts';
	import { player } from '#lib/player/player.svelte.ts';
	import { resumeWork } from '#lib/player/resume.ts';
	import { session } from '#lib/stores/session.svelte.ts';

	let pageNumber = $derived(Math.max(1, Number(page.url.searchParams.get('page')) || 1));
	let starting = $state<number | null>(null);
	let startError = $state<{ error: ApiError; work: WorkSummary } | null>(null);

	function pageHref(n: number): string {
		return n === 1 ? '/recent' : `/recent?page=${n}`;
	}

	function progressText(work: WorkSummary): string {
		const p = work.progress;
		if (!p) return '';
		if (p.finished && !p.replaying) return '已听完';
		if (!p.resumable) return '旧进度待匹配';
		const at = formatClock(p.position_ms / 1000);
		return p.track_duration_ms ? `${at} / ${formatClock(p.track_duration_ms / 1000)}` : at;
	}

	// Leaving the page abandons a start that is still waiting for the detail.
	const leaving = new AbortController();
	$effect(() => () => leaving.abort());

	async function resume(work: WorkSummary) {
		if (player.isCurrentWork(work.id)) {
			void goto('/player');
			return;
		}
		starting = work.id;
		startError = null;
		try {
			const outcome = await resumeWork(
				work.id,
				{
					getWork,
					play: (detail, start) => player.playWork(detail, start),
					sessionKey: () => session.key
				},
				leaving.signal
			);
			if (outcome === 'unplayable') {
				startError = { error: new ApiError(0, 'unknown', `《${work.title}》没有可播放的轨道`), work };
			}
		} catch (e) {
			if (leaving.signal.aborted) return;
			const error = e instanceof ApiError ? e : new ApiError(0, 'unknown', '无法开始播放');
			startError = { error, work };
		} finally {
			starting = null;
		}
	}
</script>

<svelte:head><title>最近 · kikoeru</title></svelte:head>

<LargeTitle title="最近" />

{#if startError}
	{@const failed = startError}
	<div class="pb-4">
		<LoadError error={failed.error} title="无法开始播放" onRetry={() => resume(failed.work)} />
	</div>
{/if}

<PagedWorks
	requestKey={String(pageNumber)}
	load={() => listHistory({ page: pageNumber, page_size: DEFAULT_PAGE_SIZE })}
	{pageNumber}
	{pageHref}
	errorTitle="最近播放加载失败"
>
	{#snippet empty()}
		<p class="text-[17px] font-medium">还没有播放记录</p>
		<p class="mt-1 text-[15px] text-secondary">播放过的作品会按时间倒序出现在这里。</p>
	{/snippet}
	{#snippet cardFooter(work)}
		<div class="mt-1 flex items-center gap-2">
			<button
				type="button"
				class="-ml-1 inline-flex min-h-11 items-center gap-1 rounded-lg px-1 text-[15px] font-medium text-accent-text disabled:opacity-50"
				disabled={starting !== null || work.missing}
				aria-label="继续播放《{work.title}》"
				onclick={() => resume(work)}
			>
				<Icon name="play" size={16} filled />
				{player.isCurrentWork(work.id) ? '正在播放' : '继续'}
			</button>
			<span class="min-w-0 truncate text-[13px] text-secondary tabular-nums">{progressText(work)}</span>
		</div>
	{/snippet}
</PagedWorks>
