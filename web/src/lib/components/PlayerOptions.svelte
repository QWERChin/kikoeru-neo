<script lang="ts">
	import { PLAYBACK_RATES, SLEEP_MODES, player, type SleepMode } from '../player/player.svelte';
	import { formatClock } from '../format';
	import Icon from './Icon.svelte';

	interface Props {
		/** `end` keeps the menus inside the right edge when the buttons sit there. */
		align?: 'center' | 'end';
	}

	let { align = 'center' }: Props = $props();

	let openMenu = $state<'rate' | 'sleep' | null>(null);
	let now = $state(Date.now());

	$effect(() => {
		if (player.sleepEndsAt === null) return;
		const timer = setInterval(() => (now = Date.now()), 1000);
		return () => clearInterval(timer);
	});

	function rateLabel(rate: number): string {
		return `${rate === 1 ? '1.0' : rate}×`;
	}

	function sleepLabel(mode: SleepMode): string {
		if (mode === 'off') return '关闭';
		if (mode === 'end-of-track') return '播完当前轨';
		return `${mode} 分钟`;
	}

	let sleepStatus = $derived(
		player.sleepMode === 'end-of-track'
			? '播完当前轨'
			: player.sleepEndsAt !== null
				? formatClock(Math.max(0, (player.sleepEndsAt - now) / 1000))
				: '定时关闭'
	);

	function toggle(menu: 'rate' | 'sleep') {
		openMenu = openMenu === menu ? null : menu;
	}
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && (openMenu = null)} />

<div class="relative flex items-center justify-center gap-3">
	<button
		type="button"
		class="min-h-11 min-w-16 rounded-full bg-fill px-4 text-[15px] font-semibold tabular-nums"
		aria-label="倍速 {rateLabel(player.rate)}"
		aria-expanded={openMenu === 'rate'}
		aria-controls="rate-menu"
		onclick={() => toggle('rate')}
	>
		{rateLabel(player.rate)}
	</button>
	<button
		type="button"
		class="flex min-h-11 items-center gap-1.5 rounded-full px-4 text-[15px] tabular-nums {player.sleepMode !==
		'off'
			? 'bg-accent text-on-accent'
			: 'bg-fill'}"
		aria-label="定时关闭：{player.sleepMode === 'off' ? '未开启' : sleepStatus}"
		aria-expanded={openMenu === 'sleep'}
		aria-controls="sleep-menu"
		onclick={() => toggle('sleep')}
	>
		<Icon name="moon" size={18} />
		{sleepStatus}
	</button>

	{#if openMenu === 'rate'}
		<div
			id="rate-menu"
			class="absolute bottom-full z-10 mb-2 flex gap-1 rounded-2xl bg-surface-raised p-1.5 shadow-lg {align === 'end'
				? 'right-0'
				: ''}"
			role="group"
			aria-label="选择倍速"
		>
			{#each PLAYBACK_RATES as rate (rate)}
				<button
					type="button"
					class="min-h-11 min-w-12 rounded-xl px-2 text-[15px] font-medium tabular-nums {player.rate ===
					rate
						? 'bg-accent text-on-accent'
						: 'hover:bg-fill'}"
					aria-pressed={player.rate === rate}
					onclick={() => {
						player.setRate(rate);
						openMenu = null;
					}}
				>
					{rateLabel(rate)}
				</button>
			{/each}
		</div>
	{:else if openMenu === 'sleep'}
		<div
			id="sleep-menu"
			class="absolute bottom-full z-10 mb-2 flex w-56 flex-col rounded-2xl bg-surface-raised p-1.5 shadow-lg {align ===
			'end'
				? 'right-0'
				: ''}"
			role="group"
			aria-label="定时关闭"
		>
			{#each SLEEP_MODES as mode (mode)}
				<button
					type="button"
					class="flex min-h-11 items-center justify-between rounded-xl px-3 text-left text-[15px] hover:bg-fill"
					aria-pressed={player.sleepMode === mode}
					onclick={() => {
						player.setSleep(mode);
						openMenu = null;
					}}
				>
					{sleepLabel(mode)}
					{#if player.sleepMode === mode}
						<Icon name="check" size={18} class="text-accent-text" />
					{/if}
				</button>
			{/each}
		</div>
	{/if}
</div>
