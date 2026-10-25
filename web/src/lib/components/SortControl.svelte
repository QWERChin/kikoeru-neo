<script lang="ts">
	import type { WorkSort } from '../api/query';
	import { randomSeed, type SearchState } from '../search-state';
	import Icon from './Icon.svelte';

	interface Props {
		current: SearchState;
		onChange: (patch: Partial<SearchState>) => void;
	}

	let { current, onChange }: Props = $props();

	const SORT_LABELS: Record<WorkSort, string> = {
		added: '入库时间',
		release: '发售日',
		rating: '评分',
		random: '随机'
	};
</script>

<div class="flex flex-wrap items-center gap-2">
	<label class="flex items-center gap-2 text-[15px]">
		<span class="text-secondary">排序</span>
		<select
			class="control"
			value={current.sort}
			onchange={(e) => onChange({ sort: e.currentTarget.value as WorkSort })}
		>
			{#each Object.entries(SORT_LABELS) as [value, label] (value)}
				<option {value}>{label}</option>
			{/each}
		</select>
	</label>
	{#if current.sort === 'random'}
		<button type="button" class="control" onclick={() => onChange({ seed: randomSeed() })}>
			换一批
		</button>
	{:else}
		<button
			type="button"
			class="control flex items-center gap-1"
			aria-label={current.order === 'desc' ? '降序，点击改为升序' : '升序，点击改为降序'}
			onclick={() => onChange({ order: current.order === 'desc' ? 'asc' : 'desc' })}
		>
			<Icon name="chevronDown" size={16} class={current.order === 'asc' ? 'rotate-180' : ''} />
			{current.order === 'desc' ? '降序' : '升序'}
		</button>
	{/if}
</div>

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
