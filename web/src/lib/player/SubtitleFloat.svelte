<script lang="ts">
	import Icon from '../components/Icon.svelte';
	import { player } from './player.svelte';
	import { floatLines, positionAfterDrag, subtitleFloat } from './subtitle-float.svelte';

	// The current subtitle line in a card over the pages other than the
	// player page. A video draws its subtitles on the picture, so only audio
	// gets the card. The card can be dragged anywhere in its area and stays
	// where it was let go.

	let area = $state<HTMLElement>();
	let card = $state<HTMLElement>();
	let start: { x: number; y: number; id: number } | null = null;

	let cues = $derived(player.subtitle.state === 'ready' ? player.subtitle.cues : []);
	let shown = $derived(subtitleFloat.on && player.current?.kind === 'audio' && cues.length > 0);
	let lines = $derived(floatLines(cues, player.currentTime));
	let placement = $derived.by(() => {
		const { x, y } = subtitleFloat.position;
		const drag = subtitleFloat.drag;
		return `--x:${x};--y:${y}` + (drag ? `;--dx:${drag.x}px;--dy:${drag.y}px` : '');
	});

	function onPointerDown(event: PointerEvent) {
		if (event.button !== 0 || (event.target as HTMLElement).closest('button')) return;
		start = { x: event.clientX, y: event.clientY, id: event.pointerId };
		(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
	}

	function onPointerMove(event: PointerEvent) {
		if (!start || event.pointerId !== start.id) return;
		subtitleFloat.drag = { x: event.clientX - start.x, y: event.clientY - start.y };
	}

	function onPointerUp(event: PointerEvent) {
		if (!start || event.pointerId !== start.id) return;
		start = null;
		if (subtitleFloat.drag && card && area) {
			subtitleFloat.setPosition(
				positionAfterDrag(card.getBoundingClientRect(), area.getBoundingClientRect())
			);
		}
		subtitleFloat.drag = null;
	}

	function onPointerCancel() {
		start = null;
		subtitleFloat.drag = null;
	}

	// A track change hides the card while the new subtitle loads, which can
	// happen mid-drag, before pointerup. The next card starts in its place.
	const endDragOnRemove = () => onPointerCancel;
</script>

{#if shown}
	<div class="area" bind:this={area}>
		<div
			bind:this={card}
			{@attach endDragOnRemove}
			class="card"
			class:dragging={subtitleFloat.drag !== null}
			style={placement}
			role="region"
			aria-label="悬浮字幕"
			onpointerdown={onPointerDown}
			onpointermove={onPointerMove}
			onpointerup={onPointerUp}
			onpointercancel={onPointerCancel}
			onlostpointercapture={onPointerCancel}
		>
			<span class="grip" aria-hidden="true"></span>
			<p class="current" class:faded={!lines.spoken} data-testid="float-current">
				{lines.current ?? ''}
			</p>
			<p class="next" data-testid="float-next">{lines.next ?? ''}</p>
			<button type="button" class="close" aria-label="关闭悬浮字幕" onclick={() => subtitleFloat.setOn(false)}>
				<Icon name="close" size={16} />
			</button>
		</div>
	</div>
{/if}

<style>
	/* The same area as the floating video window in VidstackHost: clear of
	   the title bar, the sidebar, the mini player and the tab bar. */
	.area {
		position: fixed;
		z-index: 30;
		top: calc(max(env(safe-area-inset-top), var(--title-bar-h)) + 12px);
		bottom: calc(49px + 72px + env(safe-area-inset-bottom) + 12px);
		left: 12px;
		right: 12px;
		pointer-events: none;
	}

	@media (min-width: 768px) {
		.area {
			top: calc(var(--title-bar-h) + 16px);
			bottom: calc(72px + 16px);
			left: calc(16rem + 16px);
			right: 16px;
		}
	}

	/* left and top place the card's corner at a fraction of the area, and the
	   translate pulls it back by the same fraction of the card's own size, so
	   x = 1 puts the right edge against the area's right edge. */
	.card {
		position: absolute;
		left: calc(var(--x) * 100%);
		top: calc(var(--y) * 100%);
		transform: translate(calc(var(--x) * -100%), calc(var(--y) * -100%))
			translate(var(--dx, 0px), var(--dy, 0px));
		width: 100%;
		padding: 0.75rem 2.75rem 0.75rem 1rem;
		border-radius: 1rem;
		/* Dark in both themes, so it reads over any cover art. */
		background: rgb(20 20 22 / 0.88);
		color: white;
		box-shadow: 0 8px 30px rgb(0 0 0 / 0.28);
		backdrop-filter: blur(20px) saturate(1.5);
		pointer-events: auto;
		cursor: grab;
		user-select: none;
		/* The browser must not scroll the page under a drag. */
		touch-action: none;
	}

	@media (min-width: 768px) {
		.card {
			width: min(380px, 100%);
		}
	}

	.card.dragging {
		cursor: grabbing;
	}

	.grip {
		position: absolute;
		top: 5px;
		left: 50%;
		width: 36px;
		height: 4px;
		margin-left: -18px;
		border-radius: 2px;
		background: rgb(255 255 255 / 0.3);
	}

	.current {
		display: -webkit-box;
		-webkit-box-orient: vertical;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		overflow: hidden;
		min-height: 26px;
		margin-top: 4px;
		font-size: 18px;
		font-weight: 600;
		line-height: 26px;
		white-space: pre-line;
		transition: opacity 0.2s;
	}

	.current.faded {
		opacity: 0.55;
	}

	.next {
		overflow: hidden;
		min-height: 20px;
		font-size: 14px;
		line-height: 20px;
		white-space: nowrap;
		text-overflow: ellipsis;
		opacity: 0.55;
	}

	/* A 44px target with a smaller visible disc. */
	.close {
		position: absolute;
		top: 0.125rem;
		right: 0.125rem;
		display: grid;
		place-items: center;
		width: 2.75rem;
		height: 2.75rem;
		border-radius: 9999px;
		color: rgb(255 255 255 / 0.7);
		cursor: pointer;
	}

	.close:hover,
	.close:focus-visible {
		color: white;
		background: radial-gradient(circle, rgb(255 255 255 / 0.15) 0 0.9rem, transparent 0.9rem);
	}

	@media (prefers-reduced-motion: reduce) {
		.current {
			transition: none;
		}
	}
</style>
