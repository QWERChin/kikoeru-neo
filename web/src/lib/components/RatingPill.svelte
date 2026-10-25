<script lang="ts">
	import Icon from './Icon.svelte';

	let { rating }: { rating: number } = $props();

	const STARS = [1, 2, 3, 4, 5];
</script>

<div
	class="absolute bottom-2 left-2 flex items-center gap-px rounded-full px-1.5 py-1"
	style="background: var(--pill-bg)"
	role="img"
	aria-label="我的评分 {rating} 星"
>
	{#each STARS as star (star)}
		<Icon
			name="star"
			size={12}
			filled
			strokeWidth={1}
			class={star <= rating ? 'text-[var(--pill-star)]' : 'text-[var(--pill-star-empty)]'}
		/>
	{/each}
</div>
