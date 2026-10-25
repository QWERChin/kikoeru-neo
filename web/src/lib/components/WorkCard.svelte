<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { WorkSummary } from '../api/types';
	import { coverStatus } from '../cover-status';
	import { favorites } from '../stores/favorites.svelte';
	import { reviews } from '../stores/reviews.svelte';
	import Cover from './Cover.svelte';
	import FavoriteButton from './FavoriteButton.svelte';
	import RatingPill from './RatingPill.svelte';
	import StatusBadges from './StatusBadges.svelte';

	let { work, footer }: { work: WorkSummary; footer?: Snippet } = $props();

	// The badge follows a toggle at once, like the heart button below it.
	let status = $derived(coverStatus({ favorite: favorites.isFavorite(work), progress: work.progress }));
	// A rating saved on the detail page shows here before the grid reloads.
	let myRating = $derived(reviews.ratingOf(work));
</script>

<article class="min-w-0">
	<a href="/works/{work.id}" class="group block rounded-xl">
		<Cover {work} size="thumb" class="rounded-xl shadow-[var(--shadow)]">
			<StatusBadges {status} />
			{#if myRating !== null}
				<RatingPill rating={myRating} />
			{/if}
			{#if status.trackFraction !== null}
				<div class="absolute inset-x-0 bottom-0 h-1 bg-black/25">
					<div class="h-full bg-listening" style="width: {status.trackFraction * 100}%"></div>
				</div>
			{/if}
		</Cover>
		<h2 class="mt-2 line-clamp-2 text-[15px] leading-5 font-medium group-hover:underline">
			{work.title}
		</h2>
	</a>
	<div class="mt-0.5 flex items-center gap-2">
		<p class="min-w-0 flex-1 truncate text-[13px] leading-[18px] text-secondary">
			{#if work.missing}<span class="text-danger">已缺失 · </span>{/if}{work.circle?.name ?? '未知社团'}
		</p>
		<FavoriteButton {work} />
	</div>
	{@render footer?.()}
</article>
