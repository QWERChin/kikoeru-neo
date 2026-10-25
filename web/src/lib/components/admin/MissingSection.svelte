<script lang="ts">
	import { cleanupMissing, getMissing } from '../../api/index';
	import { cleanupWarning, MissingStore } from '../../stores/missing.svelte';
	import { roots } from '../../stores/roots.svelte';
	import ConfirmDialog from '../ConfirmDialog.svelte';

	interface Props {
		/** Bumped by the page when a scan ends, since a scan changes what is missing. */
		version: number;
	}

	let { version }: Props = $props();

	const store = new MissingStore({ load: getMissing, cleanup: cleanupMissing });
	const dateFormat = new Intl.DateTimeFormat('zh-CN', { dateStyle: 'medium' });

	let confirming = $state(false);

	$effect(() => {
		void version;
		void store.load();
	});

	let empty = $derived(store.report !== null && store.report.works.length + store.report.locations.length === 0);
	let total = $derived(store.report ? store.report.works.length + store.report.locations.length : 0);
</script>

<section aria-labelledby="settings-missing">
	<div class="flex items-center justify-between px-1 pb-1">
		<h2 id="settings-missing" class="text-[13px] font-semibold tracking-wide text-secondary">缺失的作品和位置</h2>
		<button type="button" class="min-h-11 px-2 text-[15px] text-accent-text" onclick={() => void store.load()}>
			刷新
		</button>
	</div>
	{#if store.result}
		<p class="mb-2 rounded-xl bg-surface px-4 py-3 text-[15px]" role="status">{store.result}</p>
	{/if}
	{#if store.cleanupError}
		<div class="mb-2 flex items-center gap-3 rounded-xl bg-surface px-4 py-3" role="alert">
			<p class="flex-1 text-[15px] text-danger">{store.cleanupError}</p>
			{#if store.scanBlocked}
				<button
					type="button"
					class="min-h-11 shrink-0 rounded-lg bg-fill px-4 text-[15px] text-accent-text disabled:opacity-50"
					disabled={store.busy}
					onclick={() => void store.retry()}
				>
					重试
				</button>
			{/if}
		</div>
	{/if}
	{#if store.error}
		<p class="rounded-xl bg-surface px-4 py-3 text-[15px] text-danger" role="alert">
			缺失列表加载失败：{store.error.message}
		</p>
	{:else if !store.report}
		<div class="h-24 animate-pulse rounded-xl bg-surface" aria-busy="true" aria-label="正在加载"></div>
	{:else if empty}
		<p class="rounded-xl bg-surface px-4 py-3 text-[15px] text-secondary">没有缺失的作品和位置。</p>
	{:else}
		{@const report = store.report}
		<div class="overflow-hidden rounded-xl bg-surface">
			<label class="flex min-h-11 items-center gap-3 border-b border-separator px-4 py-2 text-[15px]">
				<input
					type="checkbox"
					class="size-5 accent-[var(--accent)]"
					checked={store.selectedCount === total}
					indeterminate={store.selectedCount > 0 && store.selectedCount < total}
					onchange={(e) => store.selectAll(e.currentTarget.checked)}
				/>
				全选
			</label>
			{#if report.works.length > 0}
				<p class="bg-fill/50 px-4 pt-2 pb-1 text-[12px] font-semibold text-secondary">
					所有位置都缺失的作品（{report.works.length}）
				</p>
				<ul aria-label="缺失的作品">
					{#each report.works as work (work.id)}
						<li class="border-b border-separator last:border-b-0">
							<label class="flex min-h-11 items-center gap-3 px-4 py-2">
								<input
									type="checkbox"
									class="size-5 shrink-0 accent-[var(--accent)]"
									checked={store.selectedWorks.includes(work.id)}
									onchange={() => store.toggleWork(work.id)}
								/>
								<span class="min-w-0 flex-1">
									<span class="block truncate">{work.title}</span>
									<span class="block text-[13px] text-secondary">
										{work.code} · {dateFormat.format(work.missing_at)} 起缺失
									</span>
								</span>
							</label>
						</li>
					{/each}
				</ul>
			{/if}
			{#if report.locations.length > 0}
				<p class="bg-fill/50 px-4 pt-2 pb-1 text-[12px] font-semibold text-secondary">
					作品还在、但这个位置缺失了（{report.locations.length}）
				</p>
				<ul aria-label="缺失的位置">
					{#each report.locations as location (location.id)}
						<li class="border-b border-separator last:border-b-0">
							<label class="flex min-h-11 items-center gap-3 px-4 py-2">
								<input
									type="checkbox"
									class="size-5 shrink-0 accent-[var(--accent)]"
									checked={store.selectedLocations.includes(location.id)}
									onchange={() => store.toggleLocation(location.id)}
								/>
								<span class="min-w-0 flex-1">
									<span class="block truncate">{location.code} · {roots.nameOf(location.root_id) ?? location.root_id}</span>
									<span class="block truncate text-[13px] text-secondary">
										{location.path} · {dateFormat.format(location.missing_at)} 起缺失
									</span>
								</span>
							</label>
						</li>
					{/each}
				</ul>
			{/if}
		</div>
		<div class="flex items-center gap-3 px-1 pt-2">
			<p class="flex-1 text-[13px] text-secondary">清理会永久删除选中的条目，不能撤销。</p>
			<button
				type="button"
				class="min-h-11 rounded-lg bg-fill px-4 font-semibold text-danger disabled:opacity-50"
				disabled={store.selectedCount === 0 || store.busy}
				onclick={() => (confirming = true)}
			>
				{store.busy ? '正在清理' : `清理所选（${store.selectedCount}）`}
			</button>
		</div>
	{/if}
</section>

<ConfirmDialog bind:open={confirming} title="清理缺失的条目？" confirmLabel="永久删除" danger onConfirm={() => void store.cleanup()}>
	<p>{cleanupWarning(store.selectedWorks.length, store.selectedLocations.length)}</p>
</ConfirmDialog>
