<script lang="ts">
	import { listTags, mergeTags } from '../../api/index';
	import { similarItems, type PickerItem } from '../../list-picker';
	import { TagMergeStore } from '../../stores/tag-merge.svelte';
	import ConfirmDialog from '../ConfirmDialog.svelte';
	import ListPicker from '../ListPicker.svelte';

	interface Props {
		/** Bumped by the page when a scan ends, since a scan can bring new tags. */
		version: number;
	}

	let { version }: Props = $props();

	const store = new TagMergeStore({ listTags, mergeTags });

	let confirming = $state(false);

	$effect(() => {
		void version;
		void store.load();
	});

	let fromTag = $derived(store.tags?.find((t) => t.id === store.from) ?? null);
	let items = $derived<PickerItem[]>((store.tags ?? []).map((t) => ({ value: String(t.id), label: t.name, work_count: t.work_count })));
	// A tag cannot be merged into itself, so the target list leaves it out.
	let targets = $derived(items.filter((t) => t.value !== String(store.from)));
	// Duplicates are usually another spelling, so those come first.
	let similar = $derived(fromTag ? similarItems(fromTag.name, targets) : []);

	const parseId = (value: string | null) => (value === null ? null : Number(value));

	function pickFrom(value: string | null) {
		store.from = parseId(value);
		// The target list leaves the source out, so a target equal to it would show as nothing.
		if (store.into === store.from) store.into = null;
	}
</script>

<section aria-labelledby="settings-tags">
	<h2 id="settings-tags" class="px-1 pb-1 text-[13px] font-semibold tracking-wide text-secondary">合并标签</h2>
	{#if store.result}
		<p class="mb-2 rounded-xl bg-surface px-4 py-3 text-[15px]" role="status">{store.result}</p>
	{/if}
	{#if store.mergeError}
		<p class="mb-2 rounded-xl bg-surface px-4 py-3 text-[15px] text-danger" role="alert">{store.mergeError}</p>
	{/if}
	{#if store.loadError}
		<p class="rounded-xl bg-surface px-4 py-3 text-[15px] text-danger" role="alert">
			标签列表加载失败：{store.loadError}
		</p>
	{:else if !store.tags}
		<div class="h-24 animate-pulse rounded-xl bg-surface" aria-busy="true" aria-label="正在加载"></div>
	{:else}
		<div class="flex flex-col gap-3 rounded-xl bg-surface px-4 py-3">
			<ListPicker
				label="要合并的标签（合并后删除）"
				noun="标签"
				{items}
				value={store.from === null ? null : String(store.from)}
				placeholder="选择标签"
				onPick={pickFrom}
			/>
			<ListPicker
				label="合并进"
				noun="标签"
				items={targets}
				value={store.into === null ? null : String(store.into)}
				placeholder="选择标签"
				suggestions={{ title: '名字相近', items: similar }}
				onPick={(v) => (store.into = parseId(v))}
			/>
			<div class="flex items-center gap-3">
				<p class="flex-1 text-[13px] text-secondary">
					{store.from !== null || store.into !== null ? (store.problem ?? '作品和别名都会转到目标标签。') : '列表只包含至少有一部作品的标签。'}
				</p>
				<button
					type="button"
					class="min-h-11 shrink-0 rounded-lg bg-accent px-4 font-semibold text-on-accent disabled:opacity-50"
					disabled={store.problem !== null || store.busy}
					onclick={() => (confirming = true)}
				>
					{store.busy ? '正在合并' : '合并'}
				</button>
			</div>
		</div>
	{/if}
</section>

<ConfirmDialog bind:open={confirming} title="合并标签？" confirmLabel="合并" onConfirm={() => void store.merge()}>
	<p>
		把「{store.nameOf(store.from)}」合并进「{store.nameOf(store.into)}」。「{store.nameOf(store.from)}」的
		{fromTag?.work_count ?? 0} 部作品和它的别名都会转到「{store.nameOf(store.into)}」，然后删除「{store.nameOf(store.from)}」。这一步不能撤销。
	</p>
</ConfirmDialog>
