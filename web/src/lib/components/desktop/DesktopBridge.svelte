<script lang="ts">
	import { onMount } from 'svelte';
	import { desktop } from '../../desktop/desktop.svelte';
	import { applyControl, lyricsOf, ShellSync, mediaStateOf } from '../../desktop/media-bridge';
	import { player } from '../../player/player.svelte';

	// Local and remote pages report what is playing to the tray and take
	// its controls. The picker plays nothing and may not listen.
	let connected = $derived(desktop.kind === 'local' || desktop.kind === 'remote');

	onMount(() => {
		void desktop.detect();
	});

	$effect(() => {
		if (!connected) return;
		let closed = false;
		let unlisten: (() => void) | null = null;
		desktop.commands.onMediaControl((control) => applyControl(control, player)).then(
			(stop) => {
				if (closed) stop();
				else unlisten = stop;
			},
			(error: unknown) => console.warn('cannot listen for the tray controls', error)
		);
		return () => {
			closed = true;
			unlisten?.();
		};
	});

	const sync = new ShellSync(desktop.commands.mediaState);
	$effect(() => {
		if (connected) sync.update(mediaStateOf(player));
	});

	// Sent whether or not the lyrics window is open, so it has the current
	// lines the moment it opens. They change about once a line.
	const lyrics = new ShellSync(desktop.commands.lyricsLines);
	$effect(() => {
		if (connected) lyrics.update(lyricsOf(player));
	});
</script>
