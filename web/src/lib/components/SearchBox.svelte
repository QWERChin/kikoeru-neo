<script lang="ts">
	import { beforeNavigate, goto } from '$app/navigation';
	import { page } from '$app/state';
	import { untrack } from 'svelte';
	import { SearchInputController } from '../search-input';
	import {
		DEFAULT_SEARCH_STATE,
		SEARCH_DEBOUNCE_MS,
		parseSearchState,
		searchHref,
		updateSearchState
	} from '../search-state';
	import { recentSearches } from '../stores/recent-searches.svelte';
	import Icon from './Icon.svelte';

	let { class: className = '', autofocus = false }: { class?: string; autofocus?: boolean } =
		$props();

	let onSearchPage = $derived(page.url.pathname === '/search');
	let urlQuery = $derived(onSearchPage ? parseSearchState(page.url.searchParams).q : '');
	let draft = $state('');

	function commit(value: string): string | null {
		const current = onSearchPage ? parseSearchState(page.url.searchParams) : DEFAULT_SEARCH_STATE;
		if (value === current.q && onSearchPage) return null;
		if (!onSearchPage && value.trim() === '') return null;
		const href = searchHref('/search', updateSearchState(current, { q: value }));
		// Typing within one search replaces the entry, so back leaves the
		// search instead of stepping through every prefix that was typed.
		const replace = onSearchPage && current.q.trim() !== '' && value.trim() !== '';
		void goto(href, { replace, reset: false });
		return href;
	}

	const controller = new SearchInputController(commit, SEARCH_DEBOUNCE_MS);

	beforeNavigate(({ to }) => {
		controller.navigationStarting(to ? `${to.url.pathname}${to.url.search}` : null);
	});

	// Follow the URL when it changes from outside, such as back or a recent
	// search, but never overwrite what is being typed.
	// Keyed on the whole URL: going back to a URL with the same q but other
	// filters still has to replace text whose search was just cancelled.
	$effect(() => {
		void page.url.href;
		const q = urlQuery;
		untrack(() => {
			const shown = controller.urlChanged(q);
			if (shown !== null) draft = shown;
		});
	});

	$effect(() => () => controller.dispose());

	function submit(event: SubmitEvent) {
		event.preventDefault();
		controller.submit(draft);
		recentSearches.remember(draft);
	}
</script>

<form role="search" onsubmit={submit} class={className}>
	<label class="relative block">
		<span class="sr-only">搜索作品</span>
		<span class="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-secondary">
			<Icon name="search" size={18} />
		</span>
		<!-- svelte-ignore a11y_autofocus -->
		<input
			type="search"
			bind:value={draft}
			placeholder="编号、标题、社团或声优"
			enterkeyhint="search"
			{autofocus}
			class="h-11 w-full rounded-lg bg-fill pr-3 pl-9 text-[15px] placeholder:text-secondary focus:outline-none focus-visible:ring-2 focus-visible:ring-accent-text"
			oninput={(e) =>
				controller.input(e.currentTarget.value, (e as unknown as InputEvent).isComposing)}
			oncompositionstart={() => controller.compositionStart()}
			oncompositionend={(e) => controller.compositionEnd(e.currentTarget.value)}
			onblur={() => {
				if (draft.trim() !== '') recentSearches.remember(draft);
			}}
		/>
	</label>
</form>
