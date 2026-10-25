<script lang="ts">
	import Icon from '../components/Icon.svelte';
	import SeekIcon from '../components/SeekIcon.svelte';
	import VolumeControl from '../components/VolumeControl.svelte';
	import { formatClock } from '../format';
	import { player } from './player.svelte';
	import { videoView, type Presentation } from './video.svelte';

	let { mode }: { mode: Presentation } = $props();

	// Drawn inside <media-player>, so it goes fullscreen with the video. The
	// player page has its own controls, so this bar only shows in fullscreen.

	/** Controls fade after this long without a pointer or key while playing. */
	const IDLE_MS = 3000;

	let idle = $state(false);
	let dragValue = $state<number | null>(null);
	let timer: ReturnType<typeof setTimeout> | null = null;

	let position = $derived(dragValue ?? player.currentTime);
	let cue = $derived(
		player.subtitle.state === 'ready' && player.activeCue >= 0
			? (player.subtitle.cues[player.activeCue]?.text ?? '')
			: ''
	);
	// JASSUB draws the ass subtitle itself. Text cues stand in only when it
	// is not used or could not start.
	let showCue = $derived(
		cue !== '' && mode !== 'thumb' && (videoView.ass.state === 'off' || videoView.ass.state === 'error')
	);
	let controlsVisible = $derived(videoView.fullscreen && (!idle || player.paused));

	function wake() {
		idle = false;
		if (timer) clearTimeout(timer);
		timer = setTimeout(() => (idle = true), IDLE_MS);
	}

	$effect(() => {
		if (!videoView.fullscreen) return;
		wake();
		return () => {
			if (timer) clearTimeout(timer);
			timer = null;
		};
	});
</script>

<div
	class="overlay"
	class:hide-cursor={videoView.fullscreen && !controlsVisible}
	onpointermove={wake}
	onpointerdown={wake}
	onkeydown={wake}
	role="presentation"
>
	{#if showCue}
		<p class="cue" class:raised={controlsVisible} class:small={mode === 'float'} data-testid="video-cue">{cue}</p>
	{/if}

	{#if videoView.fullscreen}
		<div class="bar" class:visible={controlsVisible} aria-label="全屏播放控制" role="group">
			<input
				type="range"
				class="w-full accent-white"
				min="0"
				max={Math.max(player.duration, 1)}
				step="1"
				value={position}
				aria-label="播放进度"
				aria-valuetext="{formatClock(position)} / {formatClock(player.duration)}"
				oninput={(e) => (dragValue = Number(e.currentTarget.value))}
				onchange={(e) => {
					player.seekTo(Number(e.currentTarget.value));
					dragValue = null;
				}}
			/>
			<div class="flex items-center gap-1">
				<button type="button" class="ctl" aria-label="后退 {player.seekBackSeconds} 秒" onclick={() => player.back()}>
					<SeekIcon seconds={player.seekBackSeconds} direction="back" />
				</button>
				<button
					type="button"
					class="ctl"
					aria-label={player.paused ? '播放' : '暂停'}
					onclick={() => player.toggle()}
				>
					<Icon name={player.paused ? 'play' : 'pause'} size={28} filled />
				</button>
				<button
					type="button"
					class="ctl"
					aria-label="前进 {player.seekForwardSeconds} 秒"
					onclick={() => player.forward()}
				>
					<SeekIcon seconds={player.seekForwardSeconds} direction="forward" />
				</button>
				<span class="ml-2 text-[13px] tabular-nums">
					{formatClock(position)} / {formatClock(player.duration)}
				</span>
				<span class="flex-1"></span>
				<VolumeControl variant="overlay" />
				{#if videoView.canPictureInPicture}
					<button
						type="button"
						class="ctl"
						aria-label={videoView.pictureInPicture ? '退出画中画' : '画中画'}
						onclick={() => videoView.togglePictureInPicture()}
					>
						<Icon name="pip" size={24} />
					</button>
				{/if}
				<button type="button" class="ctl" aria-label="退出全屏" onclick={() => videoView.toggleFullscreen()}>
					<Icon name="fullscreenExit" size={24} />
				</button>
			</div>
		</div>
	{/if}
</div>

<style>
	.overlay {
		position: absolute;
		inset: 0;
		/* Clicks reach the video unless they land on a control. */
		pointer-events: none;
		color: white;
	}

	:global(media-player[data-fullscreen]) .overlay {
		pointer-events: auto;
	}

	.hide-cursor {
		cursor: none;
	}

	.cue {
		position: absolute;
		left: 50%;
		bottom: 6%;
		max-width: 90%;
		transform: translateX(-50%);
		padding: 0.15em 0.5em;
		border-radius: 0.3em;
		background: rgb(0 0 0 / 0.6);
		font-size: clamp(14px, 3.2cqw, 34px);
		line-height: 1.35;
		text-align: center;
		white-space: pre-line;
		transition: bottom 0.2s;
	}

	/* The floating window is a few hundred pixels wide. */
	.cue.small {
		font-size: clamp(10px, 4cqw, 16px);
	}

	.cue.raised {
		bottom: 7.5rem;
	}

	.bar {
		position: absolute;
		inset-inline: 0;
		bottom: 0;
		padding: 2.5rem max(1rem, env(safe-area-inset-right)) max(0.75rem, env(safe-area-inset-bottom))
			max(1rem, env(safe-area-inset-left));
		background: linear-gradient(to top, rgb(0 0 0 / 0.75), transparent);
		opacity: 0;
		pointer-events: none;
		transition: opacity 0.25s;
	}

	.bar.visible {
		opacity: 1;
		pointer-events: auto;
	}

	.ctl {
		display: grid;
		place-items: center;
		width: 2.75rem;
		height: 2.75rem;
		border-radius: 9999px;
	}

	.ctl:hover {
		background: rgb(255 255 255 / 0.15);
	}
</style>
