<script lang="ts">
	import { player } from '../player/player.svelte';
	import Icon from './Icon.svelte';

	interface Props {
		/** `page` for the full player, `bar` for the mini player, `overlay` over a fullscreen video. */
		variant?: 'page' | 'bar' | 'overlay';
	}

	let { variant = 'page' }: Props = $props();

	let level = $derived(player.muted ? 0 : player.volume);
	let silent = $derived(level === 0);
</script>

<!-- iOS Safari keeps the volume on the hardware buttons, so there only the
     mute button is shown (player.volumeAdjustable). -->
<div
	class="volume is-{variant} flex items-center gap-1 {player.volumeAdjustable ? '' : 'justify-center'}"
	role="group"
	aria-label="音量控制"
>
	<button
		type="button"
		class="mute grid size-11 shrink-0 place-items-center rounded-full"
		aria-label="静音"
		aria-pressed={silent}
		onclick={() => player.toggleMute()}
	>
		<!-- On the page a full speaker already sits at the loud end of the slider. -->
		<Icon
			name={silent ? 'speakerMuted' : variant === 'page' || level < 0.5 ? 'speakerLow' : 'speaker'}
			size={22}
		/>
	</button>
	{#if player.volumeAdjustable}
		<input
			type="range"
			class="slider min-w-0 flex-1"
			min="0"
			max="1"
			step="0.01"
			value={level}
			aria-label="音量"
			aria-valuetext="{Math.round(level * 100)}%"
			style="--level: {level * 100}%"
			oninput={(e) => player.setVolume(Number(e.currentTarget.value))}
		/>
		{#if variant === 'page'}
			<Icon name="speaker" size={20} class="shrink-0 text-secondary" />
		{/if}
	{/if}
</div>

<style>
	.volume {
		--volume-fill: var(--text-secondary);
		--volume-track: var(--separator);
		color: var(--text-secondary);
	}

	.is-overlay {
		--volume-fill: white;
		--volume-track: rgb(255 255 255 / 0.3);
		color: white;
	}

	.is-page .mute,
	.is-bar .mute {
		color: var(--text-secondary);
	}

	.mute:hover {
		background: var(--fill);
	}

	.is-overlay .mute:hover {
		background: rgb(255 255 255 / 0.15);
	}

	.is-bar .slider {
		flex: none;
		width: 6rem;
	}

	.is-overlay .slider {
		flex: none;
		width: 6rem;
	}

	/* A phone held upright has no room for it in the fullscreen bar, and
	   its hardware buttons set the volume anyway. The bar is inside the
	   fullscreen <media-player>, which is the size container. */
	@container (max-width: 559px) {
		.is-overlay {
			display: none;
		}
	}

	/* Drawn like the progress slider, but in grey, as the iOS player does. */
	.slider {
		appearance: none;
		height: 2.75rem;
		background: transparent;
		cursor: pointer;
	}

	.slider::-webkit-slider-runnable-track {
		height: 4px;
		border-radius: 9999px;
		background: linear-gradient(
			to right,
			var(--volume-fill) var(--level),
			var(--volume-track) var(--level)
		);
	}

	.slider::-moz-range-track {
		height: 4px;
		border-radius: 9999px;
		background: var(--volume-track);
	}

	.slider::-moz-range-progress {
		height: 4px;
		border-radius: 9999px;
		background: var(--volume-fill);
	}

	.slider::-webkit-slider-thumb {
		appearance: none;
		width: 14px;
		height: 14px;
		margin-top: -5px;
		border-radius: 9999px;
		background: var(--volume-fill);
	}

	.slider::-moz-range-thumb {
		width: 14px;
		height: 14px;
		border: none;
		border-radius: 9999px;
		background: var(--volume-fill);
	}
</style>
