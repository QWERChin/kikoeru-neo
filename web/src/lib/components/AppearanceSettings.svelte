<script lang="ts">
	import { ACCENTS, ACCENT_LABELS, cachedAccent } from '../accent';
	import type { Accent } from '../api/types';
	import { theme } from '../stores/theme.svelte';
	import { THEME_LABELS, THEME_PREFERENCES } from '../theme';
	import { userSettings } from '../stores/user-settings.svelte';

	// Each PUT replaces every setting, so nothing is saved before the
	// account's own settings arrived. Until then the page shows the
	// remembered accent, which is what the layout applied.
	let known = $derived(userSettings.known);
	// null and "blue" look the same, so both select the blue swatch.
	let current = $derived<Accent>(known ? (userSettings.value.accent ?? 'blue') : cachedAccent());

	function pick(accent: Accent) {
		if (!known || accent === current) return;
		void userSettings.save({ ...userSettings.value, accent });
	}
</script>

<section aria-labelledby="settings-appearance">
	<h2 id="settings-appearance" class="px-1 pb-1 text-[13px] font-semibold tracking-wide text-secondary">外观</h2>
	<div class="overflow-hidden rounded-xl bg-surface">
		<div class="flex min-h-11 flex-wrap items-center gap-x-3 gap-y-2 border-b border-separator px-4 py-3">
			<span class="flex-1">主题</span>
			<!-- The same segmented control as the library type in RootSheet. -->
			<div class="flex w-full gap-1 rounded-xl bg-fill p-1 sm:w-72" role="radiogroup" aria-label="主题">
				{#each THEME_PREFERENCES as preference (preference)}
					<button
						type="button"
						role="radio"
						aria-checked={theme.preference === preference}
						class="min-h-9 flex-1 rounded-lg text-[13px] font-semibold {theme.preference === preference
							? 'bg-segment shadow-sm'
							: 'text-secondary'}"
						onclick={() => theme.set(preference)}
					>
						{THEME_LABELS[preference]}
					</button>
				{/each}
			</div>
		</div>
		<div class="px-4 py-3">
			<div class="pb-2">强调色</div>
			<div class="flex flex-wrap gap-1" role="radiogroup" aria-label="强调色">
				{#each ACCENTS as accent (accent)}
					<!-- Each swatch sets its own data-accent, so it draws in that colour
					     whatever the page's accent is (app.css). -->
					<button
						type="button"
						role="radio"
						aria-checked={accent === current}
						aria-label={ACCENT_LABELS[accent]}
						data-accent={accent}
						disabled={!known}
						class="swatch grid size-11 place-items-center rounded-full disabled:opacity-50"
						onclick={() => pick(accent)}
					>
						<span class="grid size-7 place-items-center rounded-full bg-accent">
							{#if accent === current}
								<span class="size-2.5 rounded-full bg-on-accent"></span>
							{/if}
						</span>
					</button>
				{/each}
			</div>
		</div>
	</div>
	{#if userSettings.error}
		<p class="px-1 pt-2 text-[15px] text-danger" role="alert">{userSettings.error}</p>
	{/if}
	<p class="px-1 pt-2 text-[13px] text-secondary">
		主题只记在这台设备上，选“跟随系统”时随系统的浅色和深色切换。强调色是按钮、链接和进度条的颜色，跟着账号走，在别的设备上登录也一样。
	</p>
</section>

<style>
	.swatch[aria-checked='true'] {
		box-shadow: inset 0 0 0 2px var(--accent);
	}

	.swatch:focus-visible {
		outline: 2px solid var(--accent-text);
		outline-offset: 2px;
	}
</style>
