<script lang="ts">
	import { goto } from '$app/navigation';
	import { onDestroy } from 'svelte';
	import Icon from '../components/Icon.svelte';
	import { player } from './player.svelte';
	import { nearestCorner, videoView } from './video.svelte';

	// Drawn inside <media-player> while it floats over a page other than the
	// player page. The whole picture is one button: a tap plays or pauses, a
	// drag moves the window, and it snaps to the nearest corner on release.

	/** Movement below this many pixels is still a tap. */
	const DRAG_THRESHOLD = 6;

	let start: { x: number; y: number; id: number } | null = null;
	let dragging = false;

	function onPointerDown(event: PointerEvent) {
		if (event.button !== 0) return;
		start = { x: event.clientX, y: event.clientY, id: event.pointerId };
		dragging = false;
		(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
	}

	function onPointerMove(event: PointerEvent) {
		if (!start || event.pointerId !== start.id) return;
		const x = event.clientX - start.x;
		const y = event.clientY - start.y;
		if (!dragging && Math.hypot(x, y) < DRAG_THRESHOLD) return;
		dragging = true;
		videoView.drag = { x, y };
	}

	function onPointerUp(event: PointerEvent) {
		if (!start || event.pointerId !== start.id) return;
		start = null;
		if (!dragging) {
			player.toggle();
			return;
		}
		const rect = (event.currentTarget as HTMLElement).closest('media-player')?.getBoundingClientRect();
		const area = videoView.floatArea?.getBoundingClientRect() ?? {
			left: 0,
			top: 0,
			right: innerWidth,
			bottom: innerHeight
		};
		if (rect) {
			const center = { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
			videoView.setCorner(nearestCorner(center, area));
		}
		videoView.drag = null;
	}

	// A track change or a page change can unmount the window mid-drag,
	// before pointerup. The next window must start at its corner.
	onDestroy(() => {
		videoView.drag = null;
	});

	function onPointerCancel() {
		start = null;
		dragging = false;
		videoView.drag = null;
	}
</script>

<div class="float" role="group" aria-label="视频小窗">
	<!-- Pointer taps are handled on pointerup, so a drag never toggles. A
	     click with detail 0 comes from the keyboard. -->
	<button
		type="button"
		class="surface"
		aria-label={player.paused ? '播放视频' : '暂停视频'}
		onpointerdown={onPointerDown}
		onpointermove={onPointerMove}
		onpointerup={onPointerUp}
		onpointercancel={onPointerCancel}
		onclick={(e) => e.detail === 0 && player.toggle()}
	>
		{#if player.paused}
			<span class="paused" aria-hidden="true"><Icon name="play" size={28} filled /></span>
		{/if}
	</button>
	<div class="buttons">
		<button type="button" class="ctl" aria-label="放大" onclick={() => goto('/player')}>
			<Icon name="expand" size={18} />
		</button>
		{#if videoView.canPictureInPicture}
			<button type="button" class="ctl" aria-label="画中画" onclick={() => videoView.togglePictureInPicture()}>
				<Icon name="pip" size={18} />
			</button>
		{/if}
		<button type="button" class="ctl" aria-label="关闭" onclick={() => (videoView.floatClosed = true)}>
			<Icon name="close" size={18} />
		</button>
	</div>
</div>

<style>
	.float {
		position: absolute;
		inset: 0;
		color: white;
	}

	.surface {
		position: absolute;
		inset: 0;
		display: grid;
		place-items: center;
		width: 100%;
		cursor: grab;
		/* The browser must not scroll the page under a drag. */
		touch-action: none;
	}

	.surface:active {
		cursor: grabbing;
	}

	.paused {
		display: grid;
		place-items: center;
		width: 3rem;
		height: 3rem;
		border-radius: 9999px;
		background: rgb(0 0 0 / 0.5);
	}

	.buttons {
		position: absolute;
		top: 0.25rem;
		right: 0.25rem;
		display: flex;
		gap: 0.125rem;
	}

	/* 44px targets with a smaller visible disc, so they cover less picture. */
	.ctl {
		display: grid;
		place-items: center;
		width: 2.75rem;
		height: 2.75rem;
		border-radius: 9999px;
		background: radial-gradient(circle, rgb(0 0 0 / 0.55) 0 0.95rem, transparent 0.95rem);
	}

	.ctl:hover,
	.ctl:focus-visible {
		background: radial-gradient(circle, rgb(0 0 0 / 0.8) 0 0.95rem, transparent 0.95rem);
	}
</style>
