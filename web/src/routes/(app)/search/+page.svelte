<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { ApiError, DEFAULT_PAGE_SIZE, listCircles, listTags, listVas, listWorks } from '#lib/api/index.ts';
	import type { CircleCount, TagCount, VaCount } from '#lib/api/types/index.ts';
	import Icon from '#lib/components/Icon.svelte';
	import LargeTitle from '#lib/components/LargeTitle.svelte';
	import ListPicker from '#lib/components/ListPicker.svelte';
	import type { PickerItem } from '#lib/list-picker.ts';
	import PagedWorks from '#lib/components/PagedWorks.svelte';
	import SearchBox from '#lib/components/SearchBox.svelte';
	import SortControl from '#lib/components/SortControl.svelte';
	import {
		hasFilters,
		needsSeed,
		parseSearchState,
		searchHref,
		searchStateToQuery,
		toWorksQuery,
		updateSearchState,
		type SearchState
	} from '#lib/search-state.ts';
	import { recentSearches } from '#lib/stores/recent-searches.svelte.ts';
	import { roots } from '#lib/stores/roots.svelte.ts';

	// Every control writes to the URL and the page only reads from it, so
	// refresh, back and a shared link reproduce the same search.
	let search = $derived(parseSearchState(page.url.searchParams));
	let active = $derived(hasFilters(search));

	type Lists = { tags: TagCount[]; circles: CircleCount[]; vas: VaCount[] };
	let lists = $state<Lists | null>(null);
	let listsError = $state<ApiError | null>(null);

	$effect(() => {
		if (roots.visible) void roots.load();
		let stale = false;
		Promise.all([listTags(), listCircles(), listVas()])
			.then(([tags, circles, vas]) => {
				if (!stale) lists = { tags, circles, vas };
			})
			.catch((e: unknown) => {
				if (!stale) listsError = e instanceof ApiError ? e : new ApiError(0, 'unknown', '加载失败');
			});
		return () => {
			stale = true;
		};
	});

	// A random order needs a seed before it can be paged, so a URL without
	// one is replaced by one that has it instead of adding a history entry.
	$effect(() => {
		if (needsSeed(search)) {
			void goto(searchHref('/search', updateSearchState(search, {})), {
				replace: true,
				reset: false
			});
		}
	});

	function change(patch: Partial<SearchState>) {
		void goto(searchHref('/search', updateSearchState(search, patch)), { reset: false });
	}

	let rootName = $derived(roots.nameOf(search.root));

	const toItems = (list: { id: number | string; name: string; work_count: number }[] | undefined): PickerItem[] =>
		(list ?? []).map((x) => ({ value: String(x.id), label: x.name, work_count: x.work_count }));
	// A value from a shared link that the list does not have still shows.
	const missing = (value: string) => (lists ? `已不存在（${value}）` : value);

</script>

<svelte:head><title>{search.q.trim() ? `${search.q.trim()} · 搜索` : '搜索'} · kikoeru</title></svelte:head>

{#snippet filter(
	label: string,
	value: string,
	options: { value: string; label: string }[],
	onchange: (value: string) => void
)}
	<label class="flex items-center gap-2 text-[15px]">
		<span class="text-secondary">{label}</span>
		<select class="control max-w-[11rem]" {value} onchange={(e) => onchange(e.currentTarget.value)}>
			<option value="">全部</option>
			{#each options as option (option.value)}
				<option value={option.value}>{option.label}</option>
			{/each}
			<!-- A value from a shared link that the list does not have still shows. -->
			{#if value !== '' && !options.some((o) => o.value === value)}
				<option {value}>{lists ? `已不存在（${value}）` : value}</option>
			{/if}
		</select>
	</label>
{/snippet}

<LargeTitle title="搜索" />

<div class="flex flex-col gap-3 px-4 pb-4 md:px-8">
	<!-- On desktop the sidebar's box is the search box, as the book's page table says. -->
	<SearchBox class="md:hidden" />

	<div class="flex flex-wrap items-center gap-2" role="group" aria-label="筛选">
		<ListPicker
			layout="inline"
			label="标签"
			noun="标签"
			items={toItems(lists?.tags)}
			value={search.tag === null ? null : String(search.tag)}
			placeholder="全部"
			clearLabel="全部"
			missingLabel={missing}
			onPick={(v) => change({ tag: v === null ? null : Number(v) })}
		/>
		<ListPicker
			layout="inline"
			label="社团"
			noun="社团"
			items={toItems(lists?.circles)}
			value={search.circle === null ? null : String(search.circle)}
			placeholder="全部"
			clearLabel="全部"
			missingLabel={missing}
			onPick={(v) => change({ circle: v === null ? null : Number(v) })}
		/>
		<ListPicker
			layout="inline"
			label="声优"
			noun="声优"
			items={toItems(lists?.vas)}
			value={search.va}
			placeholder="全部"
			clearLabel="全部"
			missingLabel={missing}
			onPick={(v) => change({ va: v })}
		/>
		{#if roots.visible}
			{@render filter(
				'媒体库',
				search.root ?? '',
				(roots.list ?? []).map((r) => ({ value: r.id, label: r.display_name })),
				(v) => change({ root: v === '' ? null : v })
			)}
		{/if}
	</div>
	{#if listsError}
		<p class="text-[13px] text-danger" role="alert">筛选列表加载失败：{listsError.message}</p>
	{/if}

	<SortControl current={search} onChange={change} />
</div>

{#if !active}
	<div class="px-4 md:px-8">
		{#if recentSearches.list.length > 0}
			<section aria-labelledby="recent-searches" class="md:hidden">
				<div class="flex items-center justify-between">
					<h2 id="recent-searches" class="text-[13px] font-semibold tracking-wide text-secondary">
						最近搜索
					</h2>
					<button type="button" class="min-h-11 px-2 text-[15px] text-accent-text" onclick={() => recentSearches.clear()}>
						清除
					</button>
				</div>
				<ul class="overflow-hidden rounded-xl bg-surface">
					{#each recentSearches.list as term (term)}
						<li class="border-b border-separator last:border-b-0">
							<a
								href={searchHref('/search', updateSearchState(search, { q: term }))}
								class="flex min-h-11 items-center gap-3 px-4 text-[15px]"
							>
								<Icon name="clock" size={18} class="text-secondary" />
								{term}
							</a>
						</li>
					{/each}
				</ul>
			</section>
		{/if}
		<p class="py-8 text-center text-[15px] text-secondary">
			输入编号、标题、社团或声优，或者选一个筛选条件。
		</p>
	</div>
{:else if !needsSeed(search)}
	<PagedWorks
		requestKey={searchStateToQuery(search)}
		load={() => listWorks(toWorksQuery(search, DEFAULT_PAGE_SIZE))}
		pageNumber={search.page}
		pageHref={(n) => searchHref('/search', { ...search, page: n })}
		errorTitle="搜索失败"
		{rootName}
	>
		{#snippet empty()}
			<p class="text-[17px] font-medium">没有找到作品</p>
			<p class="mt-1 text-[15px] text-secondary">换个关键词，或者去掉一些筛选条件。</p>
		{/snippet}
	</PagedWorks>
{/if}

<style>
	.control {
		min-height: 2.75rem;
		border-radius: 0.5rem;
		background: var(--surface);
		padding-inline: 0.75rem;
		font-size: 15px;
		color: var(--text);
		box-shadow: var(--shadow);
	}
</style>
