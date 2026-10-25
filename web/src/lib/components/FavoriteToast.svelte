<script lang="ts">
	import { favorites } from '../stores/favorites.svelte';
	import Icon from './Icon.svelte';

	const VISIBLE_MS = 4000;

	$effect(() => {
		const failure = favorites.failure;
		if (!failure) return;
		const timer = setTimeout(() => {
			if (favorites.failure === failure) favorites.dismissFailure();
		}, VISIBLE_MS);
		return () => clearTimeout(timer);
	});
</script>

{#if favorites.failure}
	<div
		class="fixed top-[calc(max(env(safe-area-inset-top),var(--title-bar-h))+0.75rem)] left-1/2 z-30 flex w-[min(24rem,calc(100%-2rem))] -translate-x-1/2 items-center gap-3 rounded-xl bg-surface-raised px-4 py-3 shadow-lg"
		role="alert"
	>
		<Icon name="alert" size={20} class="shrink-0 text-danger" />
		<p class="min-w-0 flex-1 text-[15px]">
			{favorites.failure.wanted ? '收藏失败' : '取消收藏失败'}：{favorites.failure.message}
		</p>
		<button
			type="button"
			class="-my-2 min-h-11 shrink-0 px-2 text-[15px] text-accent-text"
			onclick={() => favorites.dismissFailure()}
		>
			知道了
		</button>
	</div>
{/if}
