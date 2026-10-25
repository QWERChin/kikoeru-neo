<script lang="ts">
	import { desktop } from '../../desktop/desktop.svelte';
	import { shellErrorText } from '../../desktop/shell';
	import ConfirmDialog from '../ConfirmDialog.svelte';

	interface Props {
		class?: string;
	}

	let { class: className = '' }: Props = $props();

	let local = $derived(desktop.kind === 'local');
	// On success the shell replaces this window, so the page stays busy.
	let switching = $state(false);
	let switchError = $state<string | null>(null);
	let confirmSwitch = $state(false);

	async function switchMode() {
		switching = true;
		switchError = null;
		try {
			await desktop.commands.switchMode();
		} catch (error) {
			switchError = shellErrorText(error);
			switching = false;
		}
	}
</script>

<button type="button" class={className} disabled={switching} onclick={() => (confirmSwitch = true)}>
	{switching ? '正在切换…' : '切换模式'}
</button>
{#if switchError}
	<p class="px-1 pt-2 text-[15px] text-danger" role="alert">{switchError}</p>
{/if}

<ConfirmDialog bind:open={confirmSwitch} title="切换模式" confirmLabel="切换" onConfirm={() => void switchMode()}>
	{#if local}
		<p>本机媒体库会停止，窗口回到选择用法的页面，正在播放的内容会停下。</p>
	{:else}
		<p>窗口回到选择用法的页面，可以换一台服务器或者改用本机媒体库，正在播放的内容会停下。</p>
	{/if}
</ConfirmDialog>
