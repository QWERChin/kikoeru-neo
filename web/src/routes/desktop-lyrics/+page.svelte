<script lang="ts">
	import { onMount } from 'svelte';
	import type { LyricsLines, MediaControl } from '#lib/api/types/index.ts';
	import Icon from '#lib/components/Icon.svelte';
	import { desktop } from '#lib/desktop/desktop.svelte.ts';
	import { nextColor, readLyricsPrefs, resized, writeLyricsPrefs } from '#lib/desktop/lyrics-prefs.ts';
	import { connectLyrics } from '#lib/desktop/lyrics-window.ts';

	// The desktop lyrics window: a transparent, always-on-top window the shell
	// opens (book/src/desktop.md, "桌面歌词"). The playing page sends the lines
	// through the shell. Hovering shows a backdrop and the buttons; a locked
	// window lets every click through until the tray menu unlocks it.

	let lines = $state<LyricsLines | null>(null);
	let locked = $state(false);
	let prefs = $state(readLyricsPrefs());
	let error = $state<string | null>(null);

	onMount(() =>
		connectLyrics(desktop.commands, {
			lines: (next) => (lines = next),
			locked: (next) => (locked = next),
			error: (e) => (error = String(e))
		})
	);

	function save(next: typeof prefs) {
		prefs = next;
		writeLyricsPrefs(next);
	}

	function control(action: MediaControl) {
		desktop.commands.lyricsControl(action).catch((e: unknown) => (error = String(e)));
	}

	let first = $derived(lines?.current ?? (lines?.next ? null : (lines?.title ?? 'kikoeru-neo 桌面歌词')));
	let second = $derived(lines?.next ?? null);
	let faded = $derived(!lines?.current || !lines.spoken);
</script>

<svelte:head>
	<title>kikoeru-neo 桌面歌词</title>
</svelte:head>

<div class="lyrics color-{prefs.color}" class:locked style="--size: {prefs.size}px" data-tauri-drag-region>
	<div class="tools" role="toolbar" aria-label="桌面歌词">
		<button type="button" aria-label="上一轨" onclick={() => control('previous')}>
			<Icon name="previous" size={18} filled />
		</button>
		<button type="button" aria-label={lines?.playing ? '暂停' : '播放'} onclick={() => control('toggle')}>
			<Icon name={lines?.playing ? 'pause' : 'play'} size={18} filled />
		</button>
		<button type="button" aria-label="下一轨" onclick={() => control('next')}>
			<Icon name="next" size={18} filled />
		</button>
		<button type="button" aria-label="缩小字号" onclick={() => save({ ...prefs, size: resized(prefs.size, -1) })}>
			A−
		</button>
		<button type="button" aria-label="放大字号" onclick={() => save({ ...prefs, size: resized(prefs.size, 1) })}>
			A+
		</button>
		<button type="button" aria-label="换颜色" onclick={() => save({ ...prefs, color: nextColor(prefs.color) })}>
			颜色
		</button>
		<button
			type="button"
			aria-label="锁定（之后从托盘菜单解锁）"
			title="锁定后鼠标会穿过歌词，从托盘菜单的“解锁桌面歌词”解锁"
			onclick={() => desktop.commands.lyricsLock(true).catch((e: unknown) => (error = String(e)))}
		>
			锁定
		</button>
		<button type="button" aria-label="关闭桌面歌词" onclick={() => desktop.commands.lyricsToggle()}>
			<Icon name="close" size={18} />
		</button>
	</div>
	{#if error}
		<p class="first faded" data-tauri-drag-region>{error}</p>
	{:else}
		<p class="first" class:faded data-tauri-drag-region data-testid="lyrics-first">{first ?? ''}</p>
		<p class="second" data-tauri-drag-region data-testid="lyrics-second">{second ?? ''}</p>
	{/if}
</div>

<style>
	/* The window itself is transparent; only the text and the hover backdrop show. */
	:global(html:has(.lyrics)),
	:global(body:has(.lyrics)) {
		height: 100%;
		margin: 0;
		overflow: hidden;
		background: transparent;
	}

	.lyrics {
		--first: #7fc4ff;
		position: relative;
		box-sizing: border-box;
		display: flex;
		flex-direction: column;
		justify-content: flex-end;
		height: 100vh;
		padding: 0 28px 14px;
		border-radius: 18px;
		color: #f3f3f3;
		user-select: none;
		cursor: default;
		transition: background-color 0.15s;
	}

	.color-pink {
		--first: #ff8fb8;
	}

	.color-gold {
		--first: #ffd36b;
	}

	.color-white {
		--first: #ffffff;
	}

	.lyrics:not(.locked):hover {
		background: rgb(0 0 0 / 0.42);
	}

	/* Over the top of the first line, so the largest text still fits the window. */
	.tools {
		position: absolute;
		top: 8px;
		left: 50%;
		transform: translateX(-50%);
		display: flex;
		gap: 4px;
		padding: 4px 6px;
		border-radius: 12px;
		background: rgb(20 20 24 / 0.85);
		visibility: hidden;
	}

	.lyrics:not(.locked):hover .tools,
	.tools:focus-within {
		visibility: visible;
	}

	.tools button {
		display: flex;
		align-items: center;
		justify-content: center;
		min-width: 34px;
		height: 30px;
		padding: 0 8px;
		border-radius: 8px;
		color: white;
		font-size: 14px;
		cursor: pointer;
	}

	.tools button:hover,
	.tools button:focus-visible {
		background: rgb(255 255 255 / 0.15);
	}

	/* A dark outline painted under the fill keeps the text readable over
	   any window or wallpaper behind it. */
	.first,
	.second {
		overflow: hidden;
		margin: 0;
		font-weight: 700;
		white-space: nowrap;
		text-overflow: ellipsis;
		-webkit-text-stroke: 5px rgb(0 0 0 / 0.55);
		paint-order: stroke fill;
	}

	.first {
		font-size: var(--size);
		line-height: 1.4;
		color: var(--first);
		text-align: left;
	}

	.first.faded {
		opacity: 0.6;
	}

	.second {
		font-size: calc(var(--size) * 0.75);
		line-height: 1.4;
		text-align: right;
	}

	@media (prefers-reduced-motion: reduce) {
		.lyrics {
			transition: none;
		}
	}
</style>
