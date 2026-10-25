<script lang="ts">
	import { tick } from 'svelte';
	import { filterItems, highlight, sortItems, type PickerItem, type PickerOrder } from '../list-picker';
	import Icon from './Icon.svelte';
	import TitleBarDrag from './TitleBarDrag.svelte';

	interface Props {
		/** The field's name, also the sheet's title. */
		label: string;
		/** What is being picked, for the search field and the empty list: 标签, 社团. */
		noun: string;
		items: readonly PickerItem[];
		value: string | null;
		onPick: (value: string | null) => void;
		/** Shown on the trigger when nothing is picked. */
		placeholder: string;
		/** A first choice that clears the value, such as 全部 for a filter. */
		clearLabel?: string;
		/** Shown above the full list while the search is empty. */
		suggestions?: { title: string; items: readonly PickerItem[] };
		/** Label for a value the list does not hold, such as a tag from an old link. */
		missingLabel?: (value: string) => string;
		/** A label above a full-width trigger, or beside a compact one. */
		layout?: 'stacked' | 'inline';
	}

	let {
		label,
		noun,
		items,
		value,
		onPick,
		placeholder,
		clearLabel,
		suggestions,
		missingLabel = (v) => v,
		layout = 'stacked'
	}: Props = $props();

	const ORDERS: [PickerOrder, string][] = [
		['count', '按作品数'],
		['name', '按名称']
	];

	const uid = $props.id();
	let dialog = $state<HTMLDialogElement | null>(null);
	let input = $state<HTMLInputElement | null>(null);
	let open = $state(false);
	let query = $state('');
	let order = $state<PickerOrder>('count');
	let active = $state(-1);

	let picked = $derived(value === null ? null : (items.find((i) => i.value === value) ?? null));
	let triggerText = $derived(
		value === null ? placeholder : picked ? `${picked.label}（${picked.work_count}）` : missingLabel(value)
	);
	let searching = $derived(query.trim() !== '');
	let shown = $derived(sortItems(filterItems(items, query), order));
	let suggested = $derived(!searching && suggestions && suggestions.items.length > 0 ? suggestions.items : []);

	interface Row {
		value: string | null;
		item: PickerItem | null;
	}
	// One flat list for arrow-key movement, in the order the rows are drawn.
	let rows = $derived<Row[]>([
		...(clearLabel !== undefined && !searching ? [{ value: null, item: null }] : []),
		...suggested.map((item) => ({ value: item.value, item })),
		...shown.map((item) => ({ value: item.value, item }))
	]);

	async function show() {
		query = '';
		order = 'count';
		active = -1;
		open = true;
		dialog?.showModal();
		await tick();
		// Searching is the point of the sheet, and showModal would focus 取消.
		input?.focus();
	}

	function choose(next: string | null) {
		onPick(next);
		dialog?.close();
	}

	function moveTo(index: number) {
		active = index;
		void tick().then(() => document.getElementById(`${uid}-row-${index}`)?.scrollIntoView({ block: 'nearest' }));
	}

	function onKeydown(event: KeyboardEvent) {
		// Arrows and Enter belong to the IME while it composes a word. Safari sends
		// the Enter that ends a composition after compositionend, with keyCode 229.
		if (event.isComposing || event.keyCode === 229) return;
		if (event.key === 'ArrowDown' && rows.length > 0) {
			event.preventDefault();
			moveTo(Math.min(active + 1, rows.length - 1));
		} else if (event.key === 'ArrowUp' && rows.length > 0) {
			event.preventDefault();
			moveTo(Math.max(active - 1, 0));
		} else if (event.key === 'Enter') {
			event.preventDefault();
			const row = rows[active >= 0 ? active : searching ? 0 : -1];
			if (row) choose(row.value);
		}
	}
</script>

{#snippet trigger()}
	<button
		type="button"
		class={layout === 'stacked'
			? 'flex min-h-11 w-full items-center gap-2 rounded-lg bg-fill px-3 text-left text-[15px] text-text'
			: 'flex min-h-11 max-w-[11rem] items-center gap-1 rounded-lg bg-surface px-3 text-left text-[15px] text-text shadow-[var(--shadow)]'}
		aria-haspopup="dialog"
		aria-expanded={open}
		aria-labelledby="{uid}-label {uid}-value"
		onclick={() => void show()}
	>
		<span id="{uid}-value" class="min-w-0 flex-1 truncate {value === null && layout === 'stacked' ? 'text-secondary' : ''}">
			{triggerText}
		</span>
		<Icon name="chevronDown" size={16} class="shrink-0 text-secondary" />
	</button>
{/snippet}

{#if layout === 'stacked'}
	<div class="flex flex-col gap-1">
		<span id="{uid}-label" class="text-[13px] text-secondary">{label}</span>
		{@render trigger()}
	</div>
{:else}
	<div class="flex items-center gap-2 text-[15px]">
		<span id="{uid}-label" class="text-secondary">{label}</span>
		{@render trigger()}
	</div>
{/if}

<!-- A bottom sheet on phones and a centred card from md up, like the review
     sheet. A fixed height keeps the search field still while results shrink. -->
<dialog
	bind:this={dialog}
	class="mx-0 mt-auto mb-0 h-[min(88dvh,calc(100dvh-var(--title-bar-h)-0.5rem))] w-full max-w-full flex-col overflow-hidden rounded-t-2xl bg-bg p-0 text-text shadow-xl backdrop:bg-black/40 open:flex md:m-auto md:h-[min(40rem,calc(100dvh-4rem-2*var(--title-bar-h)))] md:w-[min(28rem,calc(100vw-2rem))] md:rounded-2xl"
	aria-label={label}
	onclose={() => (open = false)}
>
	<TitleBarDrag />
	<!-- Rendered only while open: a page can hold several pickers of hundreds of rows. -->
	{#if open}
	<div class="grid shrink-0 grid-cols-[1fr_auto_1fr] items-center border-b border-separator bg-surface-raised px-2">
		<button type="button" class="min-h-11 justify-self-start px-2 text-[17px] text-accent-text" onclick={() => dialog?.close()}>
			取消
		</button>
		<h2 class="py-3 text-[17px] font-semibold">{label}</h2>
	</div>

	<div class="flex shrink-0 flex-col gap-2 px-4 pt-3 pb-2">
		<div class="relative">
			<Icon name="search" size={18} class="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-secondary" />
			<input
				bind:this={input}
				type="search"
				class="min-h-11 w-full rounded-xl bg-fill pr-3 pl-10 text-[17px] text-text placeholder:text-secondary"
				placeholder="搜索{noun}"
				autocomplete="off"
				enterkeyhint="done"
				role="combobox"
				aria-expanded="true"
				aria-controls="{uid}-list"
				aria-autocomplete="list"
				aria-activedescendant={active >= 0 ? `${uid}-row-${active}` : undefined}
				bind:value={query}
				oninput={() => (active = searching ? 0 : -1)}
				onkeydown={onKeydown}
			/>
		</div>
		<div class="flex items-center gap-3">
			<div class="flex w-56 gap-1 rounded-xl bg-fill p-1" role="radiogroup" aria-label="顺序">
				{#each ORDERS as [key, text] (key)}
					<button
						type="button"
						role="radio"
						aria-checked={order === key}
						class="min-h-8 flex-1 rounded-lg text-[13px] font-semibold {order === key ? 'bg-segment shadow-sm' : 'text-secondary'}"
						onclick={() => (order = key)}
					>
						{text}
					</button>
				{/each}
			</div>
			<p class="ml-auto text-[13px] text-secondary tabular-nums" aria-live="polite">
				{searching ? `${shown.length} 个匹配` : `共 ${items.length} 个`}
			</p>
		</div>
	</div>

	<div
		id="{uid}-list"
		class="min-h-0 flex-auto space-y-4 overflow-y-auto px-4 pt-1 pb-[calc(env(safe-area-inset-bottom)+1rem)]"
		role="listbox"
		aria-label={label}
	>
		{#snippet option(row: Row, index: number)}
			{@const selected = row.value === value}
			{@const marked = row.item ? highlight(row.item.label, query) : null}
			<div
				id="{uid}-row-{index}"
				role="option"
				aria-selected={selected}
				aria-label={row.item ? `${row.item.label}（${row.item.work_count}）` : clearLabel}
				tabindex="-1"
				class="flex min-h-11 cursor-default items-center gap-3 border-b border-separator px-4 last:border-b-0 {index === active
					? 'bg-fill'
					: ''}"
				onclick={() => choose(row.value)}
				onkeydown={(e) => e.key === 'Enter' && choose(row.value)}
				onpointermove={() => (active = index)}
			>
				<span class="min-w-0 flex-1 truncate text-[17px]">
					{#if !row.item}
						{clearLabel}
					{:else if marked}
						{marked.before}<mark class="bg-transparent font-semibold text-accent-text">{marked.match}</mark>{marked.after}
					{:else}
						{row.item.label}
					{/if}
				</span>
				{#if row.item}
					<span class="shrink-0 text-[15px] text-secondary tabular-nums">{row.item.work_count}</span>
				{/if}
				<Icon name="check" size={18} class="shrink-0 text-accent-text {selected ? '' : 'invisible'}" />
			</div>
		{/snippet}

		{#if clearLabel !== undefined && !searching}
			<div class="overflow-hidden rounded-xl bg-surface">{@render option(rows[0]!, 0)}</div>
		{/if}
		{#if suggested.length > 0}
			{@const offset = clearLabel !== undefined ? 1 : 0}
			<div role="group" aria-labelledby="{uid}-suggested">
				<h3 id="{uid}-suggested" class="px-1 pb-1 text-[13px] font-semibold tracking-wide text-secondary">
					{suggestions!.title}
				</h3>
				<div class="overflow-hidden rounded-xl bg-surface">
					{#each suggested as item, i (item.value)}
						{@render option({ value: item.value, item }, offset + i)}
					{/each}
				</div>
			</div>
		{/if}
		{#if shown.length > 0}
			{@const offset = rows.length - shown.length}
			<div role="group" aria-labelledby="{uid}-all">
				{#if suggested.length > 0}
					<h3 id="{uid}-all" class="px-1 pb-1 text-[13px] font-semibold tracking-wide text-secondary">全部</h3>
				{:else}
					<span id="{uid}-all" class="sr-only">{searching ? '匹配的' : '全部'}{noun}</span>
				{/if}
				<div class="overflow-hidden rounded-xl bg-surface">
					{#each shown as item, i (item.value)}
						{@render option({ value: item.value, item }, offset + i)}
					{/each}
				</div>
			</div>
		{:else if searching}
			<p class="py-8 text-center text-[15px] text-secondary">没有匹配的{noun}</p>
		{/if}
	</div>
	{/if}
</dialog>
