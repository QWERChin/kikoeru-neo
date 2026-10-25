<script lang="ts">
	import FavoriteToast from '#lib/components/FavoriteToast.svelte';
	import MiniPlayer from '#lib/components/MiniPlayer.svelte';
	import Sidebar from '#lib/components/Sidebar.svelte';
	import TabBar from '#lib/components/TabBar.svelte';
	import { player } from '#lib/player/player.svelte.ts';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();
</script>

<div class="max-md:pt-[var(--title-bar-h)] md:flex md:h-dvh md:flex-col">
	<div class="md:flex md:min-h-0 md:flex-1">
		<div class="hidden md:block md:h-full">
			<Sidebar />
		</div>
		<!-- Phone padding clears the tab bar, and the mini player above it when shown.
		     On desktop the sidebar reaches under a macOS title bar and the main column
		     starts below it. -->
		<main
			class="min-w-0 flex-1 md:mt-[var(--title-bar-h)] md:h-[calc(100%-var(--title-bar-h))] md:overflow-y-auto md:pb-8 {player.active
				? 'pb-[calc(49px+72px+env(safe-area-inset-bottom)+1.5rem)]'
				: 'pb-[calc(49px+env(safe-area-inset-bottom)+1.5rem)]'}"
		>
			{@render children()}
		</main>
	</div>
	<div class="fixed inset-x-0 bottom-[calc(49px+env(safe-area-inset-bottom))] z-20 md:static">
		<MiniPlayer />
	</div>
	<div class="md:hidden">
		<TabBar />
	</div>
</div>
<FavoriteToast />
