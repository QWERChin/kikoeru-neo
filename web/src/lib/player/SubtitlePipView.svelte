<script lang="ts">
	import Icon from '../components/Icon.svelte';
	import { formatClock } from '../format';
	import { player } from './player.svelte';
	import { floatLines } from './subtitle-float.svelte';

	// Drawn into the browser's picture-in-picture window (subtitle-pip), which
	// stays on top of other applications. It shares this page's player.

	let cues = $derived(player.subtitle.state === 'ready' ? player.subtitle.cues : []);
	let lines = $derived(floatLines(cues, player.currentTime));
	let note = $derived(
		!player.current
			? '没有正在播放的曲目'
			: player.current.kind !== 'audio'
				? '视频的字幕显示在画面上'
				: player.subtitle.state === 'loading'
					? '正在加载字幕'
					: player.subtitle.state === 'error'
						? '字幕加载失败'
						: cues.length === 0
							? '这条轨道没有字幕'
							: null
	);
	let fraction = $derived(player.duration > 0 ? Math.min(1, player.currentTime / player.duration) : 0);
</script>

<div class="pip" role="region" aria-label="画中画字幕">
	{#if note}
		<p class="current">{player.currentTitle}</p>
		<p class="next">{note}</p>
	{:else}
		<p class="current" class:faded={!lines.spoken} data-testid="pip-current">{lines.current ?? ''}</p>
		<p class="next" data-testid="pip-next">{lines.next ?? ''}</p>
	{/if}
	<div class="controls">
		<button type="button" aria-label="上一轨" onclick={() => player.previous()}>
			<Icon name="previous" size={18} filled />
		</button>
		<button type="button" aria-label={player.paused ? '播放' : '暂停'} onclick={() => player.toggle()}>
			<Icon name={player.paused ? 'play' : 'pause'} size={20} filled />
		</button>
		<button type="button" aria-label="下一轨" disabled={!player.hasNext} onclick={() => player.next()}>
			<Icon name="next" size={18} filled />
		</button>
		<span class="track" aria-hidden="true"><span style="width: {fraction * 100}%"></span></span>
		<span class="time">{formatClock(player.currentTime)} / {formatClock(player.duration)}</span>
	</div>
</div>

<style>
	/* The window has none of the page's layout, so this sets its own base. */
	:global(html:has(.pip)),
	:global(body:has(.pip)) {
		height: 100%;
		margin: 0;
		background: #1c1c1e;
	}

	.pip {
		display: flex;
		flex-direction: column;
		box-sizing: border-box;
		height: 100%;
		padding: 14px 18px 10px;
		color: white;
		/* The font stack of app.css. */
		font-family: -apple-system, BlinkMacSystemFont, 'PingFang SC', 'Noto Sans SC', 'Noto Sans CJK SC', sans-serif;
		user-select: none;
	}

	.current {
		display: -webkit-box;
		-webkit-box-orient: vertical;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		overflow: hidden;
		margin: 0;
		font-size: 20px;
		font-weight: 700;
		line-height: 28px;
		white-space: pre-line;
		transition: opacity 0.2s;
	}

	.current.faded {
		opacity: 0.55;
	}

	.next {
		overflow: hidden;
		margin: 2px 0 0;
		font-size: 14px;
		line-height: 22px;
		white-space: nowrap;
		text-overflow: ellipsis;
		opacity: 0.55;
	}

	.controls {
		display: flex;
		align-items: center;
		gap: 4px;
		margin-top: auto;
		padding-top: 8px;
		color: rgb(255 255 255 / 0.8);
		font-size: 12px;
	}

	button {
		display: grid;
		place-items: center;
		width: 32px;
		height: 32px;
		border: 0;
		border-radius: 9999px;
		background: none;
		color: inherit;
		cursor: pointer;
	}

	button:hover,
	button:focus-visible {
		background: rgb(255 255 255 / 0.12);
		color: white;
	}

	button:disabled {
		opacity: 0.35;
		cursor: default;
		background: none;
	}

	.track {
		flex: 1;
		height: 4px;
		margin: 0 8px;
		border-radius: 2px;
		background: rgb(255 255 255 / 0.2);
		overflow: hidden;
	}

	.track span {
		display: block;
		height: 100%;
		background: var(--accent-text-dark, #5aa5f7);
	}

	.time {
		font-variant-numeric: tabular-nums;
	}

	@media (prefers-reduced-motion: reduce) {
		.current {
			transition: none;
		}
	}
</style>
