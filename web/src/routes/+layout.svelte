<script lang="ts">
	import '../app.css';
	import { applyAccent, cachedAccent } from '#lib/accent.ts';
	import { session } from '#lib/api/index.ts';
	import TitleBarDrag from '#lib/components/TitleBarDrag.svelte';
	import UpdatePrompt from '#lib/components/UpdatePrompt.svelte';
	import DesktopBridge from '#lib/components/desktop/DesktopBridge.svelte';
	import { inShell, overlayTitleBar } from '#lib/desktop/shell.ts';
	import PlaybackExtras from '#lib/player/PlaybackExtras.svelte';
	import VidstackHost from '#lib/player/VidstackHost.svelte';
	import { player } from '#lib/player/player.svelte.ts';
	import { userSettings } from '#lib/stores/user-settings.svelte.ts';
	import { theme } from '#lib/stores/theme.svelte.ts';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();

	// Set before the first render, so no page draws under the traffic lights.
	if (overlayTitleBar()) document.documentElement.dataset.titleBar = 'overlay';

	// static/theme.js already applied the theme. From here on it follows
	// the system and changes made in settings or in another tab.
	theme.start();

	// The page is drawn by this script, so the remembered accent applies
	// before anything is painted. The account's own setting replaces it once
	// it is known, which also covers a change made in the settings page.
	applyAccent(cachedAccent());
	$effect(() => {
		if (userSettings.known) applyAccent(userSettings.value.accent);
	});

	// Signing out, an expired session or another sign-in must not leave the
	// previous account's work playing, and its unsent reports must not go out
	// with the new account's token. A token the desktop shell renewed is the
	// same account, so playback goes on.
	let lastAccount = session.account;
	$effect(() => {
		const account = session.account;
		if (account === lastAccount) return;
		lastAccount = account;
		player.endSession();
	});
</script>

{@render children()}

<!-- Mounted once here so playback survives navigation between pages. -->
<VidstackHost />
<PlaybackExtras />
<UpdatePrompt />
{#if inShell()}
	<DesktopBridge />
{/if}
<TitleBarDrag cover />
