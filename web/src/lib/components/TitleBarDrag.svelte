<script lang="ts">
	import { overlayTitleBar } from '../desktop/shell';

	interface Props {
		/** Paint the page background on narrow windows, so content scrolling up hides behind it. */
		cover?: boolean;
	}

	let { cover = false }: Props = $props();

	const shown = overlayTitleBar();
</script>

<!-- The strip under the macOS traffic lights moves and zooms the window. A modal dialog makes
     the rest of the page inert, so each dialog carries its own copy, and keeps its card out of
     the strip with --title-bar-h in its maximum height. -->
{#if shown}
	<div
		data-tauri-drag-region
		data-testid={cover ? 'title-bar' : 'dialog-title-bar'}
		class="fixed inset-x-0 top-0 z-40 h-[var(--title-bar-h)] {cover ? 'max-md:bg-bg' : ''}"
	></div>
{/if}
