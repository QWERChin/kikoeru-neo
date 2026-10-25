<script lang="ts">
	import { page } from '$app/state';
	import { DEFAULT_PAGE_SIZE, listReviews } from '#lib/api/index.ts';
	import LargeTitle from '#lib/components/LargeTitle.svelte';
	import PagedWorks from '#lib/components/PagedWorks.svelte';

	let pageNumber = $derived(Math.max(1, Number(page.url.searchParams.get('page')) || 1));

	function pageHref(n: number): string {
		return n === 1 ? '/reviews' : `/reviews?page=${n}`;
	}
</script>

<svelte:head><title>我的评价 · kikoeru</title></svelte:head>

<LargeTitle title="我的评价" />

<!-- The list carries no review texts (contract), so a card's stars are all
     it shows. The text is on the detail page. -->
<PagedWorks
	requestKey={String(pageNumber)}
	load={() => listReviews({ page: pageNumber, page_size: DEFAULT_PAGE_SIZE })}
	{pageNumber}
	{pageHref}
	errorTitle="我的评价加载失败"
>
	{#snippet empty()}
		<p class="text-[17px] font-medium">还没有评价过的作品</p>
		<p class="mt-1 text-[15px] text-secondary">在作品详情页的“我的评价”里点星星打分，或者写一段短评，作品会出现在这里。</p>
	{/snippet}
</PagedWorks>
