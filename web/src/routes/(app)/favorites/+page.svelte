<script lang="ts">
	import { page } from '$app/state';
	import { DEFAULT_PAGE_SIZE, listFavorites } from '#lib/api/index.ts';
	import LargeTitle from '#lib/components/LargeTitle.svelte';
	import PagedWorks from '#lib/components/PagedWorks.svelte';

	let pageNumber = $derived(Math.max(1, Number(page.url.searchParams.get('page')) || 1));

	function pageHref(n: number): string {
		return n === 1 ? '/favorites' : `/favorites?page=${n}`;
	}
</script>

<svelte:head><title>收藏 · kikoeru</title></svelte:head>

<LargeTitle title="收藏" />

<!-- A work unfavorited here stays in the grid until the next load, so the
     heart can be tapped again to undo. -->
<PagedWorks
	requestKey={String(pageNumber)}
	load={() => listFavorites({ page: pageNumber, page_size: DEFAULT_PAGE_SIZE })}
	{pageNumber}
	{pageHref}
	errorTitle="收藏加载失败"
>
	{#snippet empty()}
		<p class="text-[17px] font-medium">还没有收藏的作品</p>
		<p class="mt-1 text-[15px] text-secondary">在作品封面下方或详情页点红心，作品会出现在这里。</p>
	{/snippet}
</PagedWorks>
