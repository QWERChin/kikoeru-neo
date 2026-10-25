<script lang="ts">
	import type { ApiError } from '../api/errors';
	import { isStorageUnavailable, STORAGE_REASSURANCE, storageUnavailableText } from '../storage-error';
	import { roots } from '../stores/roots.svelte';
	import Icon from './Icon.svelte';

	interface Props {
		error: ApiError;
		/** Heading for errors other than an unreachable media library. */
		title: string;
		/** The library the page was filtered by, which names it best. */
		rootName?: string | null;
		onRetry?: () => void;
	}

	let { error, title, rootName = null, onRetry }: Props = $props();

	let storage = $derived(isStorageUnavailable(error));

	// Admins can see which library is down, so the notice can name it even
	// when the page was not filtered by one. Fresh status is wanted here.
	$effect(() => {
		if (storage && !rootName) void roots.load(true);
	});
</script>

<div class="mx-4 flex flex-col items-start gap-3 rounded-xl bg-surface p-4 md:mx-8" role="alert">
	<p class="flex items-center gap-2 font-medium">
		<Icon name={storage ? 'cloud' : 'alert'} size={20} class={storage ? 'text-secondary' : 'text-danger'} />
		{storage ? '媒体库暂时无法访问' : title}
	</p>
	{#if storage}
		<p class="text-[15px]">{storageUnavailableText(error, { rootName, roots: roots.list })}</p>
		<p class="text-[15px] text-secondary">{STORAGE_REASSURANCE}</p>
	{:else}
		<p class="text-[15px] text-secondary">{error.message}</p>
	{/if}
	{#if onRetry}
		<button
			type="button"
			class="min-h-11 rounded-lg bg-accent px-4 font-medium text-on-accent"
			onclick={onRetry}
		>
			重试
		</button>
	{/if}
</div>
