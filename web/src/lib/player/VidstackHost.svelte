<script lang="ts">
	import { onMount, untrack } from 'svelte';
	import cjkFallback from '$fallback-fonts/fallback-cjk.woff2?url';
	import latinFallback from '$fallback-fonts/fallback-latin.woff2?url';
	import type { MediaPlayerElement } from 'vidstack/elements';
	import { getAssSubtitle, listTrackFonts } from '../api/index';
	import { ensureMediaToken, refreshMediaToken } from '../api/media';
	import { AssController } from './ass';
	import { normalizer } from './audio-extras';
	import { player, type MediaHost } from './player.svelte';
	import { presentation, slotStyle, videoAspect, videoView, type Box } from './video.svelte';
	import FloatControls from './FloatControls.svelte';
	import VideoOverlay from './VideoOverlay.svelte';
	import { SourceSequence, isSourceElementFailure } from './vidstack-source';
	import { isNativeVolumeChange, type NativeControlsProbe } from './volume';

	// The element lives in the root layout and is never unmounted, so audio
	// keeps playing across navigation and in the background (book: media.md).
	let element = $state<MediaPlayerElement>();
	let defined = $state(false);
	/** The provider's <video>, while the source is a video. */
	let video = $state<HTMLVideoElement | null>(null);
	let box = $state<Box | null>(null);

	let isVideo = $derived(player.current?.kind === 'video');
	let mode = $derived(
		presentation({
			isVideo,
			fullscreen: videoView.fullscreen,
			onPlayerPage: videoView.slot !== null,
			ready: player.load.state === 'ready',
			pictureInPicture: videoView.pictureInPicture,
			floatClosed: videoView.floatClosed,
			hasThumb: videoView.thumb !== null
		})
	);
	let aspect = $derived(videoAspect(player.current));

	/** Inline position for the modes that follow a box or a drag. */
	let placement = $derived.by(() => {
		if ((mode === 'slot' || mode === 'thumb') && box) return slotStyle(box);
		if (mode !== 'float') return undefined;
		const drag = videoView.drag;
		return `--aspect:${aspect}` + (drag ? `;transform:translate(${drag.x}px,${drag.y}px)` : '');
	});

	const ass = new AssController({
		loadSubtitle: getAssSubtitle,
		listFonts: listTrackFonts,
		mediaToken: async () => {
			try {
				return await ensureMediaToken();
			} catch {
				return null;
			}
		},
		refreshMediaToken,
		probeStatus: async (url) => {
			try {
				return (await fetch(url, { method: 'HEAD' })).status;
			} catch {
				return 0;
			}
		},
		create: async (target, content, fonts) =>
			(await import('./jassub-renderer')).createJassub(target, content, fonts),
		// Only the URLs live in this chunk. The worker downloads the fonts
		// once JASSUB starts.
		fallbackFonts: () => ({
			latin: new URL(latinFallback, location.href).href,
			cjk: new URL(cjkFallback, location.href).href
		}),
		onState: (state) => {
			videoView.ass = state;
		},
		baseUrl: () => location.href
	});

	onMount(() => {
		// Loaded on the client only; the module defines the custom elements.
		import('vidstack/player').then(() => {
			defined = true;
		});
		return () => ass.clear();
	});

	// One JASSUB instance for the current video track, on the element that
	// actually plays it. It is replaced when either changes.
	$effect(() => {
		const track = player.load.state === 'ready' ? player.current : null;
		const target = video;
		untrack(() => ass.show(track, target));
	});

	// Follows the slot on the player page, or the mini player's thumbnail:
	// its size, page scrolling and layout changes around it.
	$effect(() => {
		const slot = mode === 'slot' ? videoView.slot : mode === 'thumb' ? videoView.thumb : null;
		if (!slot) {
			box = null;
			return;
		}
		const measure = () => {
			const rect = slot.getBoundingClientRect();
			box = { top: rect.top, left: rect.left, width: rect.width, height: rect.height };
		};
		measure();
		const observer = new ResizeObserver(measure);
		observer.observe(slot);
		// A slot can move without changing size, as when the queue panel
		// narrows the column of a height-limited video. Its containers
		// change size then, so all of them are watched.
		for (let parent = slot.parentElement; parent; parent = parent.parentElement) observer.observe(parent);
		window.addEventListener('resize', measure);
		window.addEventListener('scroll', measure, { capture: true, passive: true });
		return () => {
			observer.disconnect();
			window.removeEventListener('resize', measure);
			window.removeEventListener('scroll', measure, { capture: true });
		};
	});

	// A new track floats again even if the last one's window was closed.
	$effect(() => {
		void player.current?.id;
		untrack(() => (videoView.floatClosed = false));
	});

	/** Puts a volume on a media element, leaving what already matches. */
	function applyVolume(media: HTMLMediaElement, volume = player.volume, muted = player.muted) {
		if (media.muted !== muted) media.muted = muted;
		// iOS ignores the assignment and reports 1, so writing it again would
		// only repeat a no-op on every event.
		if (player.volumeAdjustable && Math.abs(media.volume - volume) > 0.001) media.volume = volume;
	}

	/**
	 * From a new source until it plays. Vidstack writes its own volume
	 * defaults back in that time, and those changes are never the user's.
	 */
	let settling = true;

	function createHost(el: MediaPlayerElement): MediaHost {
		const sources = new SourceSequence();
		return {
			setSource(url, mimeType) {
				settling = true;
				const source = sources.next(url, mimeType);
				el.src = { src: source.src, type: source.type as 'audio/mpeg' };
			},
			clearSource() {
				sources.reset();
				el.src = '';
			},
			play: () => el.play(),
			pause() {
				void el.pause().catch(() => {});
			},
			seek(seconds) {
				el.currentTime = seconds;
			},
			setRate(rate) {
				el.playbackRate = rate;
			},
			setVolume(volume, muted) {
				// Before the first source there is no element yet. The events
				// below apply the volume once one loads.
				const media = el.querySelector('audio, video');
				if (media instanceof HTMLMediaElement) applyVolume(media, volume, muted);
			}
		};
	}

	type ProviderDetail = { type?: string; library?: unknown; video?: unknown } | null;

	$effect(() => {
		const el = element;
		if (!el) return;
		// attach() reads player state, which must not re-run this effect.
		const detach = untrack(() => player.attach(createHost(el)));
		// Only audio is normalised. A video's element is never routed, so
		// HLS and the native video players are left exactly as they were.
		normalizer.setMediaSource(() => (player.current?.kind === 'audio' ? el.querySelector('audio') : null));
		const disconnect = videoView.connect({
			async toggleFullscreen() {
				if (el.state.fullscreen) await el.exitFullscreen();
				else await el.enterFullscreen();
			},
			async togglePictureInPicture() {
				if (el.state.pictureInPicture) await el.exitPictureInPicture();
				else await el.enterPictureInPicture();
			}
		});
		const unsubscribe = el.subscribe(
			({ fullscreen, pictureInPicture, canFullscreen, canPictureInPicture }) => {
				videoView.fullscreen = fullscreen;
				videoView.pictureInPicture = pictureInPicture;
				videoView.canFullscreen = canFullscreen;
				videoView.canPictureInPicture = canPictureInPicture;
			}
		);
		const listeners: [string, (event: Event) => void][] = [
			[
				'can-play',
				() => {
					player.onDurationChange(el.duration);
					player.onCanPlay();
				}
			],
			[
				'play',
				() => {
					normalizer.resume();
					player.onPlay();
				}
			],
			['pause', () => player.onPause()],
			// The detail carries the new value; the getter can still be stale here.
			['duration-change', (e) => player.onDurationChange((e as CustomEvent<number>).detail)],
			['ended', () => player.onEnded()],
			['waiting', () => player.onWaiting()],
			['playing', () => player.onPlaying()],
			['error', () => player.onError()],
			[
				'provider-change',
				(e) => {
					const provider = (e as CustomEvent<ProviderDetail>).detail;
					// Vidstack fetches hls.js from a CDN unless told otherwise.
					// The bundled copy keeps the app self-contained. The server's
					// HLS is always one H.264 and AAC rendition in MPEG-TS, so the
					// light build, a third smaller, plays all of it. What it leaves
					// out, such as alternate audio, subtitle renditions, DRM and
					// HEVC, is listed in book/src/frontend.md.
					if (provider?.type === 'hls') provider.library = () => import('hls.js/light');
					video = provider?.video instanceof HTMLVideoElement ? provider.video : null;
				}
			]
		];
		for (const [name, listener] of listeners) el.addEventListener(name, listener);
		// Vidstack's own time-update fires on every animation frame while
		// playing. Every one rewrote the player state and restyled the
		// progress bar, about 60 times a second. The media element's
		// timeupdate comes about 4 times a second, which the second-based
		// clock and the subtitle highlight need.
		const onTimeUpdate = (event: Event) => {
			if (event.target instanceof HTMLMediaElement) player.onTimeUpdate(event.target.currentTime);
		};
		el.addEventListener('timeupdate', onTimeUpdate, true);
		// A new source resets the media element's rate, and Vidstack then
		// applies its own prop (1) after can-play, so the chosen speed would
		// last one track only. The player's rate is the only source of truth:
		// the media element's own events, caught on the way down, undo any
		// other value.
		const keepRate = (event: Event) => {
			const media = event.target;
			if (media instanceof HTMLMediaElement && media.playbackRate !== player.rate) {
				media.playbackRate = player.rate;
			}
		};
		for (const name of ['ratechange', 'playing', 'loadedmetadata']) {
			el.addEventListener(name, keepRate, true);
		}
		// The same happens to the volume: Vidstack applies its own props (1,
		// unmuted) at can-play, and a new provider brings a new element.
		// iOS native fullscreen shows the system controls instead of ours,
		// and their mute is the user's choice, so that one is kept.
		const keepVolume = (event: Event) => {
			const media = event.target;
			if (!(media instanceof HTMLMediaElement)) return;
			if (event.type === 'volumechange' && isNativeVolumeChange(media as NativeControlsProbe, settling)) {
				player.adoptVolume(media.volume, media.muted);
			} else {
				applyVolume(media);
				if (event.type === 'playing') settling = false;
			}
			// A new provider brings a new element, which joins the gain stage
			// as soon as it has loaded.
			if (media instanceof HTMLAudioElement && player.current?.kind === 'audio') normalizer.route(media);
		};
		for (const name of ['volumechange', 'playing', 'loadedmetadata']) {
			el.addEventListener(name, keepVolume, true);
		}
		// Vidstack loads typed URLs through a <source> element. A failed first
		// request (an expired media token, say) fires a non-bubbling error on
		// that element only, never on the media element Vidstack listens to,
		// so it is caught here on the way down and sent to the same recovery.
		const onSourceError = (event: Event) => {
			if (isSourceElementFailure(event.target)) player.onError();
		};
		el.addEventListener('error', onSourceError, true);
		return () => {
			el.removeEventListener('error', onSourceError, true);
			for (const name of ['volumechange', 'playing', 'loadedmetadata']) {
				el.removeEventListener(name, keepVolume, true);
			}
			for (const name of ['ratechange', 'playing', 'loadedmetadata']) {
				el.removeEventListener(name, keepRate, true);
			}
			el.removeEventListener('timeupdate', onTimeUpdate, true);
			for (const [name, listener] of listeners) el.removeEventListener(name, listener);
			unsubscribe();
			disconnect();
			detach();
			normalizer.setMediaSource(() => null);
		};
	});

	// Switching from a video to an audio track must not leave the window
	// fullscreen on a black box.
	$effect(() => {
		if (!isVideo && videoView.fullscreen) untrack(() => void videoView.toggleFullscreen());
	});
</script>

{#if defined}
	<!--
		Invisible (not display: none) while it plays audio, or a video whose
		page is not open. load="eager" because the default waits until the
		player scrolls into view, which never happens. preferNativeHLS lets
		browsers that play HLS themselves (Safari, Chrome) do so, and the
		others get the bundled hls.js.
	-->
	<media-player
		bind:this={element}
		class="media is-{mode} {mode === 'float' ? 'float-area' : ''}"
		style={placement}
		data-corner={mode === 'float' ? videoView.corner : undefined}
		load="eager"
		preload="metadata"
		playsInline
		keyDisabled
		preferNativeHLS
		aria-hidden={mode === 'hidden' ? 'true' : undefined}
		inert={mode === 'hidden'}
		data-ass={videoView.ass.state}
	>
		<media-provider></media-provider>
		{#if isVideo}
			<VideoOverlay {mode} />
		{/if}
		{#if mode === 'float'}
			<FloatControls />
		{/if}
	</media-player>
{/if}
{#if mode === 'float'}
	<!-- The area the window may sit in, for snapping to the nearest corner.
	     Outside <media-player>, whose drag transform would move it too. -->
	<div class="float-area float-probe" aria-hidden="true" {@attach videoView.attachFloatArea}></div>
{/if}

<style>
	.media {
		position: fixed;
		overflow: hidden;
	}

	.is-hidden {
		top: 0;
		left: 0;
		width: 1px;
		height: 1px;
		opacity: 0;
		pointer-events: none;
	}

	.is-slot {
		border-radius: 1rem;
		background: black;
		container-type: inline-size;
	}

	/* A small window over the page, in the corner the user dragged it to,
	   clear of the tab bar and the mini player below. */
	.float-area {
		/* Below the macOS title bar strip, which would cover the controls and take their clicks. */
		--float-top: calc(max(env(safe-area-inset-top), var(--title-bar-h)) + 12px);
		--float-bottom: calc(49px + 72px + env(safe-area-inset-bottom) + 12px);
		--float-side: 12px;
		--float-left: 12px;
		--float-width: 60vw;
	}

	@media (min-width: 768px) {
		.float-area {
			--float-top: calc(var(--title-bar-h) + 16px);
			--float-bottom: calc(72px + 16px);
			--float-side: 16px;
			/* Clear of the 16rem sidebar, so its links stay reachable. */
			--float-left: calc(16rem + 16px);
			--float-width: 400px;
		}
	}

	.is-float {
		z-index: 30;
		/* A portrait video is limited by the height, so its buttons stay on screen. */
		width: min(var(--float-width), (100dvh - var(--float-top) - var(--float-bottom)) * var(--aspect));
		aspect-ratio: var(--aspect);
		border-radius: 0.75rem;
		background: black;
		box-shadow: 0 8px 28px rgb(0 0 0 / 0.35);
		container-type: inline-size;
	}

	.float-probe {
		position: fixed;
		top: var(--float-top);
		bottom: var(--float-bottom);
		left: var(--float-left);
		right: var(--float-side);
		visibility: hidden;
		pointer-events: none;
	}

	.is-float[data-corner^='top'] {
		top: var(--float-top);
	}

	.is-float[data-corner^='bottom'] {
		bottom: var(--float-bottom);
	}

	.is-float[data-corner$='left'] {
		left: var(--float-left);
	}

	.is-float[data-corner$='right'] {
		right: var(--float-side);
	}

	/* The live picture inside the mini player's link, which still takes the tap. */
	.is-thumb {
		z-index: 30;
		border-radius: 0.375rem;
		background: black;
		pointer-events: none;
	}

	/* On phones the player page's video box runs edge to edge. */
	@media (max-width: 767px) {
		.is-slot {
			border-radius: 0;
		}
	}

	.is-fullscreen {
		background: black;
		container-type: inline-size;
	}

	.media :global(media-provider) {
		position: relative;
		display: block;
		width: 100%;
		height: 100%;
	}

	.media :global(video) {
		width: 100%;
		height: 100%;
		object-fit: contain;
	}
</style>
