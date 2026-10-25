<script lang="ts">
	import { formatClock } from '../format';
	import { trackAnalysis } from '../player/audio-extras';
	import { player } from '../player/player.svelte';
	import Waveform from './Waveform.svelte';

	interface Props {
		/** `page` puts the times under the slider, `bar` on either side of it. */
		variant?: 'page' | 'bar';
	}

	let { variant = 'page' }: Props = $props();

	// The slider shows where it is being dragged to, and seeks on release.
	let drag = $state<{ trackId: number | undefined; value: number } | null>(null);
	let position = $derived(
		drag && drag.trackId === player.current?.id ? drag.value : player.currentTime
	);
	let fraction = $derived(player.duration > 0 ? Math.min(1, position / player.duration) : 0);
	// Loaded only on desktop and only for audio (PlaybackExtras).
	let peaks = $derived(
		trackAnalysis.waveform && trackAnalysis.waveform.trackId === player.current?.id
			? trackAnalysis.waveform.peaks
			: null
	);
	let total = $derived(formatClock(player.duration));
</script>

{#snippet slider()}
	<!-- The waveform sits behind the slider, which still does the seeking.
	     It is inset by half the thumb, so its tint ends under the thumb. -->
	<div class="relative min-w-0 flex-1">
		{#if peaks}
			<Waveform
				{peaks}
				progress={fraction}
				class="absolute inset-x-[7px] top-1/2 hidden h-8 -translate-y-1/2 md:block"
			/>
		{/if}
		<!-- Replacing the input cancels the old track's native drag gesture. -->
		{#key player.current?.id}
			<input
				type="range"
				class="scrubber relative w-full"
				class:has-wave={peaks !== null}
				min="0"
				max={Math.max(player.duration, 1)}
				step="1"
				value={position}
				aria-label="播放进度"
				aria-valuetext="{formatClock(position)} / {total}"
				style="--progress: {fraction * 100}%"
				oninput={(e) => (drag = { trackId: player.current?.id, value: Number(e.currentTarget.value) })}
				onchange={(e) => {
					// Removing a focused input can commit it while the next track starts.
					if (drag?.trackId === player.current?.id) player.seekTo(Number(e.currentTarget.value));
					drag = null;
				}}
			/>
		{/key}
	</div>
{/snippet}

{#if variant === 'bar'}
	<!-- Both times are as wide as the total, so the slider does not move
	     when the elapsed time gains a digit. -->
	<div class="flex items-center gap-2 text-[12px] text-secondary tabular-nums">
		<span class="shrink-0 text-right" style="min-width: {total.length}ch">{formatClock(position)}</span>
		{@render slider()}
		<span class="shrink-0" style="min-width: {total.length}ch">{total}</span>
	</div>
{:else}
	{@render slider()}
	<div class="flex justify-between text-[13px] text-secondary tabular-nums">
		<span>{formatClock(position)}</span>
		<span>-{formatClock(Math.max(0, player.duration - position))}</span>
	</div>
{/if}

<style>
	.scrubber {
		appearance: none;
		height: 2.75rem;
		background: transparent;
		cursor: pointer;
	}

	.scrubber::-webkit-slider-runnable-track {
		height: 4px;
		border-radius: 9999px;
		background: linear-gradient(
			to right,
			var(--accent-text) var(--progress),
			var(--separator) var(--progress)
		);
	}

	.scrubber::-moz-range-track {
		height: 4px;
		border-radius: 9999px;
		background: var(--separator);
	}

	.scrubber::-moz-range-progress {
		height: 4px;
		border-radius: 9999px;
		background: var(--accent-text);
	}

	/* The waveform behind the slider shows the progress instead of the track. */
	@media (min-width: 768px) {
		.scrubber.has-wave::-webkit-slider-runnable-track {
			background: transparent;
		}

		.scrubber.has-wave::-moz-range-track {
			background: transparent;
		}

		.scrubber.has-wave::-moz-range-progress {
			background: transparent;
		}
	}

	.scrubber::-webkit-slider-thumb {
		appearance: none;
		width: 14px;
		height: 14px;
		margin-top: -5px;
		border-radius: 9999px;
		background: var(--accent-text);
	}

	.scrubber::-moz-range-thumb {
		width: 14px;
		height: 14px;
		border: none;
		border-radius: 9999px;
		background: var(--accent-text);
	}
</style>
