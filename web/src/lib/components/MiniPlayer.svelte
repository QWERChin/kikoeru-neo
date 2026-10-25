<script lang="ts">
	import { on } from 'svelte/events';
	import { player } from '../player/player.svelte';
	import { videoView } from '../player/video.svelte';
	import { onVolumeKey } from '../player/volume-keys';
	import Cover from './Cover.svelte';
	import Icon from './Icon.svelte';
	import Scrubber from './Scrubber.svelte';
	import SeekIcon from './SeekIcon.svelte';
	import VolumeControl from './VolumeControl.svelte';

	let fraction = $derived(player.duration > 0 ? Math.min(1, player.currentTime / player.duration) : 0);
	let status = $derived(
		player.load.state === 'processing'
			? '正在转码…'
			: player.load.state === 'preparing'
				? '正在准备…'
				: player.load.state === 'error'
					? player.load.message
					: (player.work?.title ?? '')
	);
</script>

{#if player.work && player.current}
	{@const work = player.work}
	<div
		class="relative border-t border-separator backdrop-blur-xl backdrop-saturate-150"
		style="background: var(--bar-bg)"
		role="region"
		aria-label="迷你播放条"
		{@attach (node) => on(node, 'keydown', onVolumeKey)}
	>
		<!-- On desktop the slider next to the title takes over from this line. -->
		<div class="absolute inset-x-0 top-0 h-0.5 bg-fill md:hidden" aria-hidden="true">
			<div class="h-full bg-accent-text" style="width: {fraction * 100}%"></div>
		</div>
		<div class="flex items-center gap-3 px-3 py-2 md:px-5">
			<a href="/player" class="flex min-w-0 flex-1 items-center gap-3" aria-label="打开播放页：{player.currentTitle}">
				{#if player.current.kind === 'video' && videoView.floatClosed}
					<!-- The playing video itself is drawn over this box (VidstackHost)
					     once its floating window is closed. -->
					<span
						{@attach videoView.attachThumb}
						class="aspect-[4/3] w-14 shrink-0 rounded-md bg-black"
						data-testid="video-thumb"
					></span>
				{:else}
					<Cover {work} size="thumb" eager iconSize={20} class="w-14 shrink-0 rounded-md" />
				{/if}
				<span class="min-w-0">
					<span class="block truncate text-[15px] font-medium">{player.currentTitle}</span>
					<span class="block truncate text-[13px] text-secondary">{status}</span>
				</span>
			</a>
			<!-- Phones have no room for it and seek on the player page. -->
			<div class="hidden min-w-48 flex-1 md:block">
				<Scrubber variant="bar" />
			</div>
			<!-- Phones have room for two buttons only, play and next. -->
			<button
				type="button"
				class="hidden size-11 shrink-0 place-items-center rounded-full hover:bg-fill md:grid"
				aria-label="后退 {player.seekBackSeconds} 秒"
				onclick={() => player.back()}
			>
				<SeekIcon seconds={player.seekBackSeconds} direction="back" size={28} />
			</button>
			<button
				type="button"
				class="grid size-11 shrink-0 place-items-center rounded-full hover:bg-fill"
				aria-label={player.paused ? '播放' : '暂停'}
				onclick={() => player.toggle()}
			>
				<Icon name={player.paused ? 'play' : 'pause'} size={26} filled />
			</button>
			<button
				type="button"
				class="hidden size-11 shrink-0 place-items-center rounded-full hover:bg-fill md:grid"
				aria-label="前进 {player.seekForwardSeconds} 秒"
				onclick={() => player.forward()}
			>
				<SeekIcon seconds={player.seekForwardSeconds} direction="forward" size={28} />
			</button>
			<button
				type="button"
				class="grid size-11 shrink-0 place-items-center rounded-full hover:bg-fill disabled:opacity-40"
				aria-label="下一轨"
				disabled={!player.hasNext}
				onclick={() => player.next()}
			>
				<Icon name="next" size={24} filled />
			</button>
			<!-- Phones change the volume with their buttons, and the bar has no room. -->
			<div class="hidden md:block">
				<VolumeControl variant="bar" />
			</div>
		</div>
	</div>
{/if}
