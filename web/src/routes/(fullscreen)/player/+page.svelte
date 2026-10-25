<script lang="ts">
	import { goto } from '$app/navigation';
	import Cover from '#lib/components/Cover.svelte';
	import Icon from '#lib/components/Icon.svelte';
	import PlayerOptions from '#lib/components/PlayerOptions.svelte';
	import Scrubber from '#lib/components/Scrubber.svelte';
	import SeekIcon from '#lib/components/SeekIcon.svelte';
	import VolumeControl from '#lib/components/VolumeControl.svelte';
	import { formatClock } from '#lib/format.ts';
	import { player } from '#lib/player/player.svelte.ts';
	import { trackTitle } from '#lib/player/queue.ts';
	import { videoAspect, videoView } from '#lib/player/video.svelte.ts';
	import { onVolumeKey } from '#lib/player/volume-keys.ts';

	let panel = $state<'subtitle' | 'queue'>('subtitle');
	/** The side panel with the queue next to a video, on desktop. */
	let queueOpen = $state(false);
	let cueList = $state<HTMLOListElement>();
	let panelBody = $state<HTMLDivElement>();

	let waiting = $derived(player.load.state === 'preparing' || player.load.state === 'processing');
	let isVideo = $derived(player.current?.kind === 'video');
	let aspect = $derived(videoAspect(player.current));

	function close() {
		if (history.length > 1) history.back();
		else void goto('/library');
	}

	// Keeps the spoken line in view without fighting a user who scrolls away:
	// only the panel scrolls, and only when the line changes.
	$effect(() => {
		const index = player.activeCue;
		if (index < 0 || !cueList || !panelBody) return;
		const item = cueList.children[index] as HTMLElement | undefined;
		if (!item) return;
		// scrollIntoView would also scroll the page on phones, so only the
		// panel moves.
		const panel = panelBody;
		const top = item.offsetTop - (panel.clientHeight - item.clientHeight) / 2;
		panel.scrollTo({ top, behavior: 'smooth' });
	});
</script>

<svelte:head><title>{player.currentTitle || '正在播放'} · kikoeru</title></svelte:head>

<!-- The whole page is the player, so the arrow keys reach it from anywhere,
     the fullscreen video included. -->
<svelte:window onkeydown={onVolumeKey} />

{#snippet transport()}
	<div class="flex items-center justify-between gap-1 px-2 md:justify-center md:gap-5">
		<button
			type="button"
			class="grid size-14 place-items-center rounded-full hover:bg-fill"
			aria-label="后退 {player.seekBackSeconds} 秒"
			onclick={() => player.back()}
		>
			<SeekIcon seconds={player.seekBackSeconds} direction="back" />
		</button>
		<button
			type="button"
			class="grid size-14 place-items-center rounded-full hover:bg-fill disabled:opacity-40"
			aria-label="上一轨"
			onclick={() => player.previous()}
		>
			<Icon name="previous" size={30} filled />
		</button>
		<button
			type="button"
			class="grid size-18 place-items-center rounded-full bg-accent text-on-accent shadow-md"
			aria-label={player.paused ? '播放' : '暂停'}
			onclick={() => player.toggle()}
		>
			<Icon name={player.paused ? 'play' : 'pause'} size={34} filled />
		</button>
		<button
			type="button"
			class="grid size-14 place-items-center rounded-full hover:bg-fill disabled:opacity-40"
			aria-label="下一轨"
			disabled={!player.hasNext}
			onclick={() => player.next()}
		>
			<Icon name="next" size={30} filled />
		</button>
		<button
			type="button"
			class="grid size-14 place-items-center rounded-full hover:bg-fill"
			aria-label="前进 {player.seekForwardSeconds} 秒"
			onclick={() => player.forward()}
		>
			<SeekIcon seconds={player.seekForwardSeconds} direction="forward" />
		</button>
	</div>
{/snippet}

{#snippet loadOverlay()}
	{#if waiting || player.load.state === 'error'}
		<div
			class="absolute inset-0 grid place-items-center bg-black/55 p-6 text-center text-white"
			role="status"
		>
			{#if player.load.state === 'error'}
				<div class="flex flex-col items-center gap-3">
					<Icon name="alert" size={32} />
					<p class="text-[15px]">{player.load.message}</p>
					<button
						type="button"
						class="min-h-11 rounded-full bg-white/20 px-5 font-medium"
						onclick={() => player.play()}
					>
						重试
					</button>
				</div>
			{:else if player.load.state === 'processing'}
				<div class="flex flex-col items-center gap-2">
					<span class="spinner" aria-hidden="true"></span>
					<p class="text-[15px] font-medium">正在转码，完成后自动播放</p>
					<p class="text-[13px] text-white/80 tabular-nums">
						{player.load.progress === null
							? '等待中'
							: `${Math.round(player.load.progress * 100)}%`}
						· 可以切到别的轨道
					</p>
				</div>
			{:else}
				<span class="spinner" aria-label="正在准备"></span>
			{/if}
		</div>
	{/if}
{/snippet}

{#snippet subtitlePanel()}
	{#if player.subtitle.state === 'ready'}
		{@const cues = player.subtitle.cues}
		<ol bind:this={cueList} class="flex flex-col gap-4 py-6 md:py-[40vh]" aria-label="字幕">
			{#each cues as cue, i (i)}
				{@const active = i === player.activeCue}
				<li>
					<button
						type="button"
						class="w-full rounded-lg px-2 py-1 text-left text-[20px] leading-7 font-semibold whitespace-pre-line transition-opacity md:text-[24px] md:leading-8 {active
							? 'opacity-100'
							: 'opacity-35 hover:opacity-60'}"
						aria-current={active ? 'true' : undefined}
						onclick={() => player.seekTo(cue.start)}
					>
						{cue.text}
					</button>
				</li>
			{/each}
		</ol>
	{:else}
		<p class="grid h-full min-h-40 place-items-center text-center text-[15px] text-secondary">
			{#if player.subtitle.state === 'loading'}
				正在加载字幕
			{:else if player.subtitle.state === 'error'}
				字幕加载失败
			{:else}
				这条轨道没有字幕
			{/if}
		</p>
	{/if}
{/snippet}

{#snippet queuePanel()}
	<ol class="overflow-hidden rounded-xl bg-surface" aria-label="播放队列">
		{#each player.queue as track, i (track.id)}
			{@const current = i === player.index}
			<li class="border-b border-separator last:border-b-0">
				<button
					type="button"
					class="flex min-h-11 w-full items-center gap-3 px-4 py-2.5 text-left hover:bg-fill {current
						? 'font-semibold text-accent-text'
						: ''}"
					aria-current={current ? 'true' : undefined}
					onclick={() => player.jumpTo(i)}
				>
					<span class="w-6 shrink-0 text-right text-[13px] text-secondary tabular-nums">
						{#if current}<Icon name="speaker" size={16} class="inline" />{:else}{i + 1}{/if}
					</span>
					<span class="min-w-0 flex-1 text-[15px] break-words">{trackTitle(track.path)}</span>
					{#if track.duration_ms !== null}
						<span class="shrink-0 text-[13px] text-secondary tabular-nums">
							{formatClock(track.duration_ms / 1000)}
						</span>
					{/if}
				</button>
			</li>
		{/each}
	</ol>
{/snippet}

{#snippet videoButtons()}
	{#if videoView.canPictureInPicture}
		<button
			type="button"
			class="grid size-11 shrink-0 place-items-center rounded-full text-secondary hover:bg-fill"
			aria-label={videoView.pictureInPicture ? '退出画中画' : '画中画'}
			aria-pressed={videoView.pictureInPicture}
			onclick={() => videoView.togglePictureInPicture()}
		>
			<Icon name="pip" size={24} />
		</button>
	{/if}
	{#if videoView.canFullscreen}
		<button
			type="button"
			class="grid size-11 shrink-0 place-items-center rounded-full text-secondary hover:bg-fill"
			aria-label="全屏"
			onclick={() => videoView.toggleFullscreen()}
		>
			<Icon name="fullscreen" size={24} />
		</button>
	{/if}
{/snippet}

{#snippet videoTitle(work: { id: number; title: string })}
	<div class="min-w-0 flex-1">
		<h1 class="truncate text-[22px] leading-7 font-bold md:text-[17px] md:leading-6" title={player.currentTitle}>
			{player.currentTitle}
		</h1>
		<a href="/works/{work.id}" class="block truncate text-[17px] text-accent-text md:text-[13px]">
			{work.title}
		</a>
	</div>
{/snippet}

{#if isVideo && player.work && player.current}
	{@const work = player.work}
	<!-- A video gets the width of the window. Its subtitles are drawn on the
	     picture, so the page has no subtitle tab. The queue opens in a side
	     panel on desktop and sits below the controls on phones. -->
	<main class="player-page flex min-h-dvh flex-col md:h-dvh md:flex-row">
		<section
			class="flex min-w-0 flex-1 flex-col pt-[max(env(safe-area-inset-top),var(--title-bar-h),0.5rem)] pb-[calc(env(safe-area-inset-bottom)+1rem)] md:h-dvh md:min-h-0 md:pb-4"
		>
			<header class="flex items-center gap-2 px-5 md:px-6">
				<button
					type="button"
					class="grid size-11 shrink-0 place-items-center rounded-full text-secondary hover:bg-fill"
					aria-label="关闭播放页"
					onclick={close}
				>
					<Icon name="chevronDown" size={28} />
				</button>
				<p class="flex-1 text-center text-[13px] font-semibold tracking-wide text-secondary md:hidden">正在播放</p>
				<div class="hidden min-w-0 flex-1 items-center gap-1 md:flex">
					{@render videoTitle(work)}
					{@render videoButtons()}
				</div>
				<button
					type="button"
					class="hidden size-11 shrink-0 place-items-center rounded-full hover:bg-fill md:grid {queueOpen
						? 'bg-fill text-accent-text'
						: 'text-secondary'}"
					aria-label="播放队列"
					aria-expanded={queueOpen}
					aria-controls={queueOpen ? 'video-queue' : undefined}
					onclick={() => (queueOpen = !queueOpen)}
				>
					<Icon name="list" size={24} />
				</button>
				<span class="size-11 shrink-0 md:hidden" aria-hidden="true"></span>
			</header>

			<!-- The shared media element is drawn over the box (VidstackHost), so
			     the page only reserves the space and shows the waiting state
			     before the source is ready. On desktop the box is the largest
			     one at the video's aspect ratio that fits above the controls. -->
			<div class="video-area mt-2 md:min-h-0 md:flex-1 md:px-6">
				<div class="video-fit" style="--aspect: {aspect}">
					<div
						{@attach videoView.attachSlot}
						class="relative w-full overflow-hidden bg-black md:rounded-2xl"
						style="aspect-ratio: {aspect}"
						data-testid="video-slot"
					>
						{@render loadOverlay()}
					</div>
				</div>
			</div>

			<div class="controls-box mx-auto mt-4 w-full max-w-md shrink-0 px-5 md:mt-3 md:max-w-6xl md:px-6">
				<div class="flex items-start gap-1 md:hidden">
					{@render videoTitle(work)}
					<div class="-my-2 flex">{@render videoButtons()}</div>
				</div>
				<div class="mt-5 md:mt-0"><Scrubber /></div>
				<div class="video-controls mt-3 md:mt-1">
					<div class="px-1"><VolumeControl /></div>
					<div>{@render transport()}</div>
					<div><PlayerOptions align="end" /></div>
				</div>
			</div>

			<div class="mt-6 px-5 md:hidden" data-own-arrows>{@render queuePanel()}</div>
		</section>

		{#if queueOpen}
			<aside
				id="video-queue"
				class="hidden w-96 shrink-0 flex-col border-l border-separator px-5 pt-[calc(1.5rem+var(--title-bar-h))] pb-4 md:flex md:h-dvh"
				aria-label="播放队列面板"
			>
				<div class="min-h-0 flex-1 overflow-y-auto" data-own-arrows>{@render queuePanel()}</div>
			</aside>
		{/if}
	</main>
{:else}
<main class="player-page mx-auto flex min-h-dvh max-w-[90rem] flex-col md:h-dvh md:flex-row">
	<section class="flex flex-col px-5 pt-[max(env(safe-area-inset-top),var(--title-bar-h),0.5rem)] pb-4 md:h-dvh md:min-h-0 md:w-1/2 md:px-12">
		<header class="flex items-center justify-between">
			<button
				type="button"
				class="grid size-11 place-items-center rounded-full text-secondary hover:bg-fill"
				aria-label="关闭播放页"
				onclick={close}
			>
				<Icon name="chevronDown" size={28} />
			</button>
			<p class="text-[13px] font-semibold tracking-wide text-secondary">正在播放</p>
			<span class="size-11" aria-hidden="true"></span>
		</header>

		{#if !player.work || !player.current}
			<div class="flex flex-1 flex-col items-center justify-center gap-3 py-24 text-center">
				<p class="text-[17px] font-medium">现在没有在播放</p>
				<a href="/library" class="inline-flex min-h-11 items-center text-accent-text">去资料库挑一部作品</a>
			</div>
		{:else}
			{@const work = player.work}
			<!-- On desktop the cover and controls sit together in the middle of
			     the column, so the space is not all left between them. -->
			<div class="md:flex md:min-h-0 md:flex-1 md:flex-col md:justify-center md:pb-6">
				<!-- On desktop the cover takes only the height the controls leave, so
				     the page fits the window instead of scrolling. -->
				<div class="cover-slot audio-slot mx-auto mt-4 w-full max-w-md md:max-w-lg">
					<div class="cover-fit">
					<Cover
						{work}
						size="full"
						eager
						iconSize={64}
						class="w-full rounded-2xl shadow-xl transition-transform duration-300 {player.paused
							? 'scale-95'
							: 'scale-100'}"
					>
						{@render loadOverlay()}
					</Cover>
					</div>
				</div>

			<div class="mx-auto mt-6 w-full max-w-md shrink-0 md:max-w-lg">
				<div class="flex items-start gap-2">
					<h1 class="min-w-0 flex-1 truncate text-[22px] leading-7 font-bold" title={player.currentTitle}>
						{player.currentTitle}
					</h1>
				</div>
				<a href="/works/{work.id}" class="mt-0.5 block truncate text-[17px] text-accent-text">
					{work.title}
				</a>

				<div class="mt-5"><Scrubber /></div>

				<div class="mt-3">{@render transport()}</div>
				<div class="mt-2 px-1"><VolumeControl /></div>
				<div class="mt-4"><PlayerOptions /></div>
			</div>
			</div>
		{/if}
	</section>

	{#if player.work && player.current}
		<section class="flex min-h-[50vh] flex-col px-5 pb-[calc(env(safe-area-inset-bottom)+1rem)] md:h-dvh md:w-1/2 md:border-l md:border-separator md:px-10 md:pt-[calc(1.5rem+var(--title-bar-h))]">
			<div class="flex justify-center gap-1 rounded-xl bg-fill p-1" role="tablist" aria-label="播放页面板">
				{#each [['subtitle', '字幕'], ['queue', '播放队列']] as [id, label] (id)}
					<button
						type="button"
						role="tab"
						class="min-h-9 flex-1 rounded-lg text-[13px] font-semibold {panel === id
							? 'bg-segment shadow-sm'
							: 'text-secondary'}"
						aria-selected={panel === id}
						onclick={() => (panel = id as 'subtitle' | 'queue')}
					>
						{label}
					</button>
				{/each}
			</div>
			<div
				bind:this={panelBody}
				class="relative mt-3 max-h-[60vh] min-h-0 flex-1 overflow-y-auto md:max-h-none"
				role="tabpanel"
			>
				{#if panel === 'subtitle'}
					{@render subtitlePanel()}
				{:else}
					{@render queuePanel()}
				{/if}
			</div>
		</section>
	{/if}
</main>
{/if}

<style>
	.player-page {
		background: var(--bg);
	}

	@media (min-width: 768px) {
		.cover-slot {
			flex: 1 1 0;
			min-height: 0;
			container-type: size;
			display: grid;
			place-items: center;
		}

		/* A cover keeps its full 4:3 height when there is room and shrinks
		   only when the window is too low. */
		.audio-slot {
			flex: 0 1 auto;
			aspect-ratio: 4 / 3;
		}

		/* The largest 4:3 box that fits the slot in both directions. */
		.cover-fit {
			width: min(100cqw, 100cqh * 4 / 3);
		}

		/* The same for a video, at its own aspect ratio. */
		.video-fit {
			width: min(100cqw, 100cqh * var(--aspect));
		}

		.video-area {
			container-type: size;
			display: grid;
			place-items: center;
		}
	}

	/* The video controls follow the width left to them, which the queue
	   panel narrows, rather than the window. */
	.controls-box {
		container-type: inline-size;
	}

	/* Stacked as for audio, transport first. */
	.video-controls {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
	}

	.video-controls > :nth-child(2) {
		order: -1;
	}

	/* One row when the transport and both side columns fit: volume,
	   transport and options, so the picture keeps the height. */
	@container (min-width: 820px) {
		.video-controls {
			display: grid;
			grid-template-columns: 1fr auto 1fr;
			align-items: center;
			gap: 1rem;
		}

		.video-controls > :nth-child(2) {
			order: 0;
		}

		.video-controls > :first-child {
			max-width: 16rem;
		}

		.video-controls > :last-child {
			justify-self: end;
		}
	}

	.spinner {
		display: inline-block;
		width: 2rem;
		height: 2rem;
		border-radius: 9999px;
		border: 3px solid rgb(255 255 255 / 0.3);
		border-top-color: white;
		animation: spin 0.9s linear infinite;
	}

	@keyframes spin {
		to {
			transform: rotate(360deg);
		}
	}
</style>
