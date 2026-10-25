<script lang="ts">
	import type { WorkSummary } from '../api/types';
	import { favorites } from '../stores/favorites.svelte';
	import Icon from './Icon.svelte';

	interface Props {
		work: Pick<WorkSummary, 'id' | 'favorite' | 'title'>;
		/** `large` is the detail page's square button; `inline` sits in a card. */
		variant?: 'large' | 'inline';
	}

	let { work, variant = 'inline' }: Props = $props();

	let on = $derived(favorites.isFavorite(work));
	let pending = $derived(favorites.isPending(work.id));
</script>

<!-- A constant name with aria-pressed reads as a toggle in screen readers. -->
<button
	type="button"
	class="grid shrink-0 place-items-center disabled:opacity-60 {variant === 'large'
		? 'size-12 rounded-xl bg-surface'
		: '-m-2.5 size-11 rounded-full hover:bg-fill'} {on ? 'text-heart' : 'text-secondary'}"
	aria-label="收藏"
	aria-pressed={on}
	title={on ? `取消收藏《${work.title}》` : `收藏《${work.title}》`}
	disabled={pending}
	onclick={() => void favorites.toggle(work)}
>
	<Icon name="heart" size={variant === 'large' ? 24 : 20} filled={on} />
</button>
