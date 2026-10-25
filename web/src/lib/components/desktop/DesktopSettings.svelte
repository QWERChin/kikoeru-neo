<script lang="ts">
	import { desktop } from '../../desktop/desktop.svelte';
	import OpenLogsButton from './OpenLogsButton.svelte';
	import SwitchModeButton from './SwitchModeButton.svelte';

	let local = $derived(desktop.kind === 'local');
</script>

{#if local || desktop.kind === 'remote'}
	<section aria-labelledby="settings-desktop">
		<h2 id="settings-desktop" class="px-1 pb-1 text-[13px] font-semibold tracking-wide text-secondary">桌面应用</h2>
		<div class="overflow-hidden rounded-xl bg-surface">
			<div class="flex min-h-11 items-center gap-3 border-b border-separator px-4 py-3">
				<span class="flex-1">模式</span>
				<span class="text-secondary">{local ? '本机媒体库' : '连接服务器'}</span>
			</div>
			<div class="flex min-h-11 items-center gap-3 border-b border-separator px-4 py-3">
				<span class="shrink-0">服务器地址</span>
				<span class="min-w-0 flex-1 truncate text-right text-secondary" data-testid="desktop-server-url">
					{#if !local}
						{location.origin}
					{:else}
						{desktop.state?.server_url ?? '没有保存的服务器'}
					{/if}
				</span>
			</div>
			<!-- A remote page may not open folders on this computer. -->
			{#if local}
				<OpenLogsButton
					class="flex min-h-11 w-full items-center border-b border-separator px-4 py-3 text-left text-accent-text hover:bg-fill"
				/>
			{/if}
			<SwitchModeButton
				class="flex min-h-11 w-full items-center px-4 py-3 text-left text-accent-text hover:bg-fill disabled:opacity-50"
			/>
		</div>
	</section>
{/if}
