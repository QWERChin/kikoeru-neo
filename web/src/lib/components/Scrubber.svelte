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

	// The slider moves the thumb's centre between half a thumb (12px) from
	// either end, so the waveform and the pointer maths are inset by that much.
	const THUMB_INSET_PX = 6;
	let hover = $state<number | null>(null);

	function trackHover(e: PointerEvent & { currentTarget: HTMLInputElement }) {
		const box = e.currentTarget.getBoundingClientRect();
		const fraction = (e.clientX - box.left - THUMB_INSET_PX) / (box.width - 2 * THUMB_INSET_PX);
		hover = Math.min(1, Math.max(0, fraction));
	}
</script>

{#snippet slider()}
	<!-- The waveform stands just above the slider's track, clear of the
	     thumb, and the slider still does the seeking. It is inset by half
	     the thumb, so its tint ends above the thumb's centre. -->
	<div class="relative min-w-0 flex-1">
		{#if peaks}
			<Waveform
				{peaks}
				progress={fraction}
				{hover}
				class="absolute inset-x-[6px] bottom-[calc(50%+6px)] hidden h-[18px] md:block"
			/>
		{/if}
		<!-- Replacing the input cancels the old track's native drag gesture. -->
		{#key player.current?.id}
			<input
				type="range"
				class="scrubber relative w-full"
				min="0"
				max={Math.max(player.duration, 1)}
				step="1"
				value={position}
				aria-label="播放进度"
				aria-valuetext="{formatClock(position)} / {total}"
				style="--progress: {fraction * 100}%"
				onpointermove={trackHover}
				onpointerleave={() => (hover = null)}
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

	/* In the text colour, so it stands out from the accent fill and the
	   waveform's tint. The ring of background parts it from the bars above. */
	.scrubber::-webkit-slider-thumb {
		appearance: none;
		width: 12px;
		height: 12px;
		margin-top: -4px;
		border-radius: 9999px;
		background: var(--text);
		box-shadow: 0 0 0 3px color-mix(in srgb, var(--bg) 70%, transparent);
	}

	.scrubber::-moz-range-thumb {
		width: 12px;
		height: 12px;
		border: none;
		border-radius: 9999px;
		background: var(--text);
		box-shadow: 0 0 0 3px color-mix(in srgb, var(--bg) 70%, transparent);
	}
</style>
