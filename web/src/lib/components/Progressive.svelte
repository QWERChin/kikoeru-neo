<script lang="ts" generics="T">
	import type { Snippet } from 'svelte';

	interface Props {
		items: readonly T[];
		key: (item: T) => string | number;
		row: Snippet<[T]>;
	}

	let { items, key, row }: Props = $props();

	/**
	 * Rows drawn at once, then per frame. A folder of the real library can
	 * hold ~700 files, which took 400 ms to open on a phone-speed CPU when
	 * drawn in one go (book: frontend.md, "长列表").
	 */
	const FIRST = 60;
	const STEP = 120;

	let shown = $state(FIRST);
	$effect(() => {
		if (shown >= items.length) return;
		const frame = requestAnimationFrame(() => (shown += STEP));
		return () => cancelAnimationFrame(frame);
	});
</script>

{#each items.length > shown ? items.slice(0, shown) : items as item (key(item))}
	{@render row(item)}
{/each}
