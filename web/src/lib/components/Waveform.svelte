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
	let count = $derived(barCount(width));
	let path = $derived(count > 0 && height > 0 ? barsPath(foldPeaks(peaks, count), width, height) : '');
	// The faint bars run between the playhead and the pointer, whichever
	// side the pointer is on, so their edge at the pointer shows where a
	// click would seek to.
	let played = $derived(clamp(progress));
	let pointer = $derived(hover === null ? null : clamp(hover));
	let tint = $derived(percent(pointer === null ? played : Math.min(played, pointer)));
	let preview = $derived(pointer === null ? null : percent(Math.max(played, pointer)));

	function clamp(fraction: number): number {
		return Math.min(1, Math.max(0, fraction));
	}

	function percent(fraction: number): string {
		return `${fraction * 100}%`;
	}
</script>

<div
	class="wave {className}"
	bind:clientWidth={width}
	bind:clientHeight={height}
	style:--tint={tint}
	style:--preview={preview}
	aria-hidden="true"
	data-testid="waveform"
	data-bars={count}
>
	{#if path}
		<!-- The viewBox is the box in CSS pixels, so the bars are not stretched. -->
		<svg class="layer base" viewBox="0 0 {width} {height}">
			<path d={path} stroke-width={BAR_WIDTH_PX} />
		</svg>
		{#if preview}
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

	.preview {
		color: color-mix(in srgb, var(--accent-text) 45%, transparent);
		clip-path: inset(0 calc(100% - var(--preview)) 0 0);
	}

	.tint {
		color: var(--accent-text);
		clip-path: inset(0 calc(100% - var(--tint)) 0 0);
	}
</style>
