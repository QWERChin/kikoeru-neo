<script lang="ts">
	import { untrack, type Snippet } from 'svelte';
	import { ApiError } from '../api/errors';
	import type { Page, WorkSummary } from '../api/types';
	import { favorites } from '../stores/favorites.svelte';
	import { reviews } from '../stores/reviews.svelte';
	import Icon from './Icon.svelte';
	import LoadError from './LoadError.svelte';
	import WorkCard from './WorkCard.svelte';

	interface Props {
		/** Changes whenever the request changes; a new key fetches again. */
		requestKey: string;
		load: () => Promise<Page<WorkSummary>>;
		pageNumber: number;
		pageHref: (n: number) => string;
		errorTitle: string;
		rootName?: string | null;
		/** Shown when the result has no works at all. */
		empty: Snippet;
		/** Extra content under a card, such as the recent page's continue button. */
		cardFooter?: Snippet<[WorkSummary]>;
	}

	let {
		requestKey,
		load,
		pageNumber,
		pageHref,
		errorTitle,
		rootName = null,
		empty,
		cardFooter
	}: Props = $props();

	let result = $state<Page<WorkSummary> | null>(null);
	let error = $state<ApiError | null>(null);
	let loading = $state(true);
	let attempt = $state(0);

	let pageCount = $derived(result ? Math.max(1, Math.ceil(result.total / result.page_size)) : 1);

	$effect(() => {
		void requestKey;
		void attempt;
		let stale = false;
		loading = true;
		error = null;
		const mark = favorites.mark();
		const reviewMark = reviews.mark();
		untrack(load)
			.then((data) => {
				if (stale) return;
				favorites.reconcile(data.items, mark);
				reviews.reconcile(data.items, reviewMark);
				result = data;
			})
			.catch((e: unknown) => {
				if (stale) return;
				error = e instanceof ApiError ? e : new ApiError(0, 'unknown', '加载失败');
			})
			.finally(() => {
				if (!stale) loading = false;
			});
		// A newer request supersedes this one, so its late answer is ignored.
		return () => {
			stale = true;
		};
	});
</script>

{#if error}
	<LoadError {error} title={errorTitle} {rootName} onRetry={() => attempt++} />
{:else if loading && !result}
	<ul class="work-grid" aria-busy="true" aria-label="正在加载">
		{#each { length: 8 }, i (i)}
			<li aria-hidden="true">
				<div class="aspect-[4/3] animate-pulse rounded-xl bg-fill"></div>
				<div class="mt-2 h-4 w-4/5 animate-pulse rounded bg-fill"></div>
				<div class="mt-1.5 h-3 w-1/2 animate-pulse rounded bg-fill"></div>
			</li>
		{/each}
	</ul>
{:else if result && result.items.length === 0}
	<div class="px-4 py-12 text-center md:px-8">
		{#if result.total === 0}
			{@render empty()}
		{:else}
			<p class="text-[17px] font-medium">这一页没有作品</p>
			<p class="mt-1 text-[15px] text-secondary">
				<a href={pageHref(1)} class="inline-flex min-h-11 items-center text-accent-text">回到第一页</a>
			</p>
		{/if}
	</div>
{:else if result}
	<ul class="work-grid" class:opacity-60={loading} aria-busy={loading}>
		{#each result.items as work (work.id)}
			<li>
				{#if cardFooter}
					<WorkCard {work}>
						{#snippet footer()}{@render cardFooter(work)}{/snippet}
					</WorkCard>
				{:else}
					<WorkCard {work} />
				{/if}
			</li>
		{/each}
	</ul>

	<nav class="flex items-center justify-center gap-4 px-4 pt-8" aria-label="分页">
		{#if pageNumber > 1}
			<a href={pageHref(pageNumber - 1)} class="pager-button" aria-label="上一页">
				<Icon name="chevronLeft" size={20} />
			</a>
		{:else}
			<button type="button" class="pager-button" aria-label="上一页" disabled>
				<Icon name="chevronLeft" size={20} />
			</button>
		{/if}
		<span class="min-w-24 text-center text-[15px] text-secondary tabular-nums">
			第 {pageNumber} / {pageCount} 页 · 共 {result.total} 部
		</span>
		{#if pageNumber < pageCount}
			<a href={pageHref(pageNumber + 1)} class="pager-button" aria-label="下一页">
				<Icon name="chevronRight" size={20} />
			</a>
		{:else}
			<button type="button" class="pager-button" aria-label="下一页" disabled>
				<Icon name="chevronRight" size={20} />
			</button>
		{/if}
	</nav>
{/if}

<style>
	.work-grid {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: 1.25rem 0.75rem;
		padding-inline: 1rem;
	}

	@media (min-width: 768px) {
		.work-grid {
			/* 12rem fits five to six covers per row at common desktop widths. */
			grid-template-columns: repeat(auto-fill, minmax(12rem, 1fr));
			gap: 1.75rem 1.25rem;
			padding-inline: 2rem;
		}
	}

	.pager-button {
		display: inline-grid;
		place-items: center;
		width: 2.75rem;
		height: 2.75rem;
		border-radius: 9999px;
		background: var(--surface);
		color: var(--accent-text);
		box-shadow: var(--shadow);
	}

	.pager-button:disabled {
		color: var(--text-secondary);
		opacity: 0.5;
	}
</style>
