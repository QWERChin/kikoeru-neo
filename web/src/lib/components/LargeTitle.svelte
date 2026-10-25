<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		title: string;
		/** Controls shown at the right of the title row, such as a pager. */
		actions?: Snippet;
	}

	let { title, actions }: Props = $props();
</script>

<header class="flex items-end justify-between gap-4 px-4 pt-6 pb-3 md:px-8 md:pt-10">
	<h1 class="text-[34px] leading-[41px] font-bold tracking-tight">{title}</h1>
	{#if actions}
		<div class="flex items-center gap-2 pb-1">{@render actions()}</div>
	{/if}
</header>
