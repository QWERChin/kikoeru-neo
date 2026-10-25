<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { DEFAULT_PAGE_SIZE, listWorks } from '#lib/api/index.ts';
	import LargeTitle from '#lib/components/LargeTitle.svelte';
	import PagedWorks from '#lib/components/PagedWorks.svelte';
	import SortControl from '#lib/components/SortControl.svelte';
	import {
		needsSeed,
		parseSearchState,
		searchHref,
		searchStateToQuery,
		sortOnly,
		toWorksQuery,
		updateSearchState,
		type SearchState
	} from '#lib/search-state.ts';

	// Sort and page live in the URL so back, refresh and shared links all
	// land on the same page. The library has no filters; /search has them.
	let search = $derived(sortOnly(parseSearchState(page.url.searchParams)));

	// A random order needs a seed before it can be paged, so a URL without
	// one is replaced by one that has it instead of adding a history entry.
	$effect(() => {
		if (needsSeed(search)) {
			void goto(searchHref('/library', updateSearchState(search, {})), {
				replace: true,
				reset: false
			});
		}
	});

	function change(patch: Partial<SearchState>) {
		void goto(searchHref('/library', updateSearchState(search, patch)), { reset: false });
	}
</script>

<svelte:head><title>资料库 · kikoeru</title></svelte:head>

<LargeTitle title="资料库" />

<div class="px-4 pb-4 md:px-8">
	<SortControl current={search} onChange={change} />
</div>

{#if !needsSeed(search)}
	<PagedWorks
		requestKey={searchStateToQuery(search)}
		load={() => listWorks(toWorksQuery(search, DEFAULT_PAGE_SIZE))}
		pageNumber={search.page}
		pageHref={(n) => searchHref('/library', { ...search, page: n })}
		errorTitle="作品列表加载失败"
	>
		{#snippet empty()}
			<p class="text-[17px] font-medium">资料库里还没有作品</p>
			<p class="mt-1 text-[15px] text-secondary">扫描媒体库之后，作品会出现在这里。</p>
		{/snippet}
	</PagedWorks>
{/if}
