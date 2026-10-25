<script lang="ts">
	import { page } from '$app/state';
	import { TAB_ITEMS, isActive } from '../navigation';
	import Icon from './Icon.svelte';
</script>

<nav
	class="fixed inset-x-0 bottom-0 z-20 border-t border-separator backdrop-blur-xl backdrop-saturate-150"
	style="background: var(--bar-bg); padding-bottom: env(safe-area-inset-bottom)"
	aria-label="主导航"
>
	<ul class="grid grid-cols-5">
		{#each TAB_ITEMS as item (item.href)}
			{@const active = isActive(page.url.pathname, item.href)}
			<li>
				<a
					href={item.href}
					class="flex min-h-[49px] flex-col items-center justify-center gap-0.5 text-[10px] font-medium {active
						? 'text-accent-text'
						: 'text-secondary'}"
					aria-current={active ? 'page' : undefined}
				>
					<Icon name={item.icon} size={24} filled={active && item.icon === 'heart'} />
					{item.label}
				</a>
			</li>
		{/each}
	</ul>
</nav>
