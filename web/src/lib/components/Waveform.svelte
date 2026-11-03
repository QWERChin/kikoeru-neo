<script lang="ts">
	import { BAR_WIDTH_PX, barCount, barsPath, foldPeaks } from '../player/waveform';

	interface Props {
		/** 0 to 1, one per thousandth of the track. */
		peaks: Float32Array;
		/** The played part, 0 to 1. */
		progress: number;
		/** Where the pointer would seek to, 0 to 1, or null while it is elsewhere. */
		hover?: number | null;
		class?: string;
	}

	let { peaks, progress, hover = null, class: className = '' }: Props = $props();

	let width = $state(0);
	let height = $state(0);
	// The path depends on the box size only, so playback never rebuilds it.
	// Progress moves a clip on the tinted copy, which costs a repaint of
	// the clipped layer and no layout (book: frontend.md, the progress bar).
	let count = $derived(barCount(width, peaks.length));
	let path = $derived(count > 0 && height > 0 ? barsPath(foldPeaks(peaks, count), width, height) : '');
	let played = $derived(percent(progress));
	let hovered = $derived(hover === null ? null : percent(hover));

	function percent(fraction: number): string {
		return `${Math.min(1, Math.max(0, fraction)) * 100}%`;
	}
</script>

<div
	class="wave {className}"
	bind:clientWidth={width}
	bind:clientHeight={height}
	style:--played={played}
	style:--hover={hovered}
	aria-hidden="true"
	data-testid="waveform"
	data-bars={count}
>
	{#if path}
		<!-- The viewBox is the box in CSS pixels, so the bars are not stretched. -->
		<svg class="layer base" viewBox="0 0 {width} {height}">
			<path d={path} stroke-width={BAR_WIDTH_PX} />
		</svg>
		{#if hovered}
			<svg class="layer preview" viewBox="0 0 {width} {height}">
				<path d={path} stroke-width={BAR_WIDTH_PX} />
			</svg>
		{/if}
		<svg class="layer tint" viewBox="0 0 {width} {height}">
			<path d={path} stroke-width={BAR_WIDTH_PX} />
		</svg>
	{/if}
</div>

<style>
	/* The caller positions the box (relative or absolute), because this
	   unlayered rule would override Tailwind's layered utilities. */
	.wave {
		pointer-events: none;
	}

	.layer {
		position: absolute;
		inset: 0;
		width: 100%;
		height: 100%;
		fill: none;
		stroke: currentColor;
		stroke-linecap: round;
	}

	.base {
		color: color-mix(in srgb, var(--text-secondary) 45%, transparent);
	}

	/* Up to the pointer, so the bars show where a click would seek to. */
	.preview {
		color: color-mix(in srgb, var(--accent-text) 45%, transparent);
		clip-path: inset(0 calc(100% - var(--hover)) 0 0);
	}

	.tint {
		color: var(--accent-text);
		clip-path: inset(0 calc(100% - var(--played)) 0 0);
	}
</style>
