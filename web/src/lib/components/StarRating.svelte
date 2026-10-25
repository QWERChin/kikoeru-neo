<script lang="ts">
	import Icon from './Icon.svelte';

	interface Props {
		value: number | null;
		onPick: (star: number) => void;
		disabled?: boolean;
		label?: string;
	}

	let { value, onPick, disabled = false, label = '评分' }: Props = $props();

	const STARS = [1, 2, 3, 4, 5];
</script>

<!-- aria-pressed marks the current rating, whose star clears it when tapped again. -->
<div class="flex" role="group" aria-label={label}>
	{#each STARS as star (star)}
		{@const filled = value !== null && star <= value}
		<button
			type="button"
			class="grid size-11 place-items-center rounded-lg disabled:opacity-60 {filled
				? 'text-accent-text'
				: 'text-star-empty'}"
			aria-label="评 {star} 星"
			aria-pressed={value === star}
			{disabled}
			onclick={() => onPick(star)}
		>
			<Icon name="star" size={26} filled strokeWidth={1.2} />
		</button>
	{/each}
</div>
