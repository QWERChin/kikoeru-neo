<script lang="ts">
	import type { CoverStatus } from '../cover-status';
	import Icon from './Icon.svelte';

	let { status }: { status: CoverStatus } = $props();

	const RING_RADIUS = 8;
	const RING_CIRCUMFERENCE = 2 * Math.PI * RING_RADIUS;

	// Without a known track fraction the ring is drawn closed, which still
	// reads as "in progress" next to the check and the heart.
	let ringOffset = $derived(
		status.trackFraction === null ? 0 : RING_CIRCUMFERENCE * (1 - status.trackFraction)
	);

	let label = $derived(
		[
			status.favorite && '已收藏',
			status.finished && '听过',
			status.listening && (status.finished ? '重听中' : '在听')
		]
			.filter(Boolean)
			.join('，')
	);
</script>

{#if status.favorite || status.finished || status.listening}
	<div
		class="absolute top-2 right-2 flex items-center gap-1 rounded-full px-1.5 py-1"
		style="background: var(--badge-bg)"
		role="img"
		aria-label={label}
	>
		{#if status.listening}
			<svg width="18" height="18" viewBox="0 0 20 20" class="text-listening" aria-hidden="true">
				<circle cx="10" cy="10" r={RING_RADIUS} fill="none" stroke="currentColor" stroke-opacity="0.25" stroke-width="2.5" />
				<circle
					cx="10"
					cy="10"
					r={RING_RADIUS}
					fill="none"
					stroke="currentColor"
					stroke-width="2.5"
					stroke-linecap="round"
					stroke-dasharray={RING_CIRCUMFERENCE}
					stroke-dashoffset={ringOffset}
					transform="rotate(-90 10 10)"
				/>
			</svg>
		{/if}
		{#if status.finished}
			<Icon name="check" size={18} strokeWidth={2.6} class="text-finished" />
		{/if}
		{#if status.favorite}
			<Icon name="heart" size={18} filled class="text-heart" />
		{/if}
	</div>
{/if}
