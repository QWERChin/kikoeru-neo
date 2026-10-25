<script lang="ts">
	import { barCount, barsPath, foldPeaks } from '../player/waveform';

	interface Props {
		/** 0 to 1, one per thousandth of the track. */
		peaks: Float32Array;
		/** The played part, 0 to 1. */
		progress: number;
		class?: string;
	}

	let { peaks, progress, class: className = '' }: Props = $props();

	let width = $state(0);
	// The path depends on the width only, so playback never rebuilds it.
	// Progress moves a clip on the tinted copy, which costs a repaint of
	// the clipped layer and no layout (book: frontend.md, the progress bar).
	let count = $derived(barCount(width, peaks.length));
	let path = $derived(count > 0 ? barsPath(foldPeaks(peaks, count)) : '');
	let played = $derived(`${Math.min(1, Math.max(0, progress)) * 100}%`);
</script>

<div
	class="wave {className}"
	bind:clientWidth={width}
	style:--played={played}
	aria-hidden="true"
	data-testid="waveform"
	data-bars={count}
>
	{#if path}
		<svg class="layer base" viewBox="0 0 {count} 100" preserveAspectRatio="none">
			<path d={path} />
		</svg>
		<svg class="layer tint" viewBox="0 0 {count} 100" preserveAspectRatio="none">
			<path d={path} />
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
		fill: currentColor;
	}

	.base {
		color: var(--separator);
	}

	.tint {
		color: var(--accent-text);
		clip-path: inset(0 calc(100% - var(--played)) 0 0);
	}
</style>
