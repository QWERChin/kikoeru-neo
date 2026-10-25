<script lang="ts">
	import type { Snippet } from 'svelte';
	import { coverToken } from '../api/cover-cookie';
	import { coverUrl, refreshMediaToken, type CoverSize } from '../api/media';
	import { session } from '../stores/session.svelte';
	import type { WorkSummary } from '../api/types';
	import { coverHue } from '../cover-color';
	import Icon from './Icon.svelte';

	interface Props {
		work: Pick<WorkSummary, 'id' | 'has_cover' | 'cover_version'>;
		size: CoverSize;
		/** Below-the-fold grids load lazily; the detail page loads at once. */
		eager?: boolean;
		class?: string;
		iconSize?: number;
		children?: Snippet;
	}

	let { work, size, eager = false, class: className = '', iconSize = 36, children }: Props =
		$props();

	// State is keyed by the cover it belongs to, because the mini player and
	// the detail page reuse this component when the work changes.
	let key = $derived(`${work.id}:${work.cover_version}:${size}`);
	let failedKey = $state<string | null>(null);
	let retriedKey = $state<string | null>(null);
	let loadedSrc = $state<string | null>(null);

	const cookiesEnabled = typeof navigator === 'undefined' || navigator.cookieEnabled;
	let src = $derived(
		work.has_cover && failedKey !== key
			? coverUrl(
					work,
					size,
					coverToken(session.mediaToken, retriedKey === key, cookiesEnabled)
				)
			: null
	);
	let loaded = $derived(src !== null && loadedSrc === src);

	// A media token can expire while the page is open. One refresh is tried,
	// and the retry carries the new token in its URL, before falling back to
	// the placeholder, so a broken cover cannot loop.
	async function onError() {
		const failing = key;
		if (retriedKey === failing) {
			failedKey = failing;
			return;
		}
		try {
			await refreshMediaToken();
		} catch {
			if (key === failing) failedKey = failing;
			return;
		}
		if (key === failing) retriedKey = failing;
	}
</script>

<div
	class="relative aspect-[4/3] overflow-hidden {className}"
	style="background-color: hsl({coverHue(work.id)} var(--cover-saturation) var(--cover-lightness))"
>
	{#if src}
		<img
			{src}
			alt=""
			loading={eager ? 'eager' : 'lazy'}
			decoding="async"
			class="absolute inset-0 h-full w-full object-cover transition-opacity duration-200 {loaded
				? 'opacity-100'
				: 'opacity-0'}"
			onload={() => (loadedSrc = src)}
			onerror={onError}
		/>
	{:else}
		<div class="absolute inset-0 grid place-items-center text-white/80">
			<Icon name="music" size={iconSize} strokeWidth={1.5} />
		</div>
	{/if}
	{@render children?.()}
</div>
