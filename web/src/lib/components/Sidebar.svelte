<script lang="ts">
	import { page } from '$app/state';
	import { SIDEBAR_ITEMS, isActive, type NavItem } from '../navigation';
	import { roots } from '../stores/roots.svelte';
	import Icon from './Icon.svelte';
	import RootHealthDot from './RootHealthDot.svelte';
	import SearchBox from './SearchBox.svelte';

	const SETTINGS: NavItem = { href: '/settings', label: '设置', icon: 'settings' };

	// Only admins may read /api/admin/roots, so listeners see no library list.
	$effect(() => {
		if (roots.visible) void roots.load();
	});

	let activeRoot = $derived(
		page.url.pathname === '/search' ? page.url.searchParams.get('root') : null
	);
</script>

{#snippet navLink(item: NavItem)}
	{@const active = isActive(page.url.pathname, item.href)}
	<a
		href={item.href}
		class="flex min-h-11 items-center gap-3 rounded-lg px-3 text-[15px] {active
			? 'bg-accent font-semibold text-on-accent'
			: 'hover:bg-fill'}"
		aria-current={active ? 'page' : undefined}
	>
		<Icon name={item.icon} size={20} class={active ? '' : 'text-accent-text'} />
		{item.label}
	</a>
{/snippet}

<!-- Under a macOS title bar the background reaches the top, and the list scrolls below it,
     so nothing scrolls under the traffic lights. -->
<nav
	class="flex h-full w-64 flex-col border-r border-separator bg-surface pt-[var(--title-bar-h)]"
	aria-label="主导航"
>
	<div class="flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto px-3 py-4">
		<SearchBox />

		<ul class="flex flex-col gap-0.5">
			{#each SIDEBAR_ITEMS as item (item.href)}
				<li>{@render navLink(item)}</li>
			{/each}
			<li>
				<!-- No browse page for circles and voice actors is scheduled yet; the
				     search filters cover them. -->
				<span class="flex min-h-11 items-center gap-3 rounded-lg px-3 text-[15px] text-secondary">
					<Icon name="people" size={20} />
					社团与声优<span class="sr-only">（尚未开放）</span>
				</span>
			</li>
		</ul>

		{#if roots.visible && roots.list && roots.list.length > 0}
			<section aria-labelledby="sidebar-libraries">
				<h2
					id="sidebar-libraries"
					class="px-3 pb-1 text-[13px] font-semibold tracking-wide text-secondary"
				>
					媒体库
				</h2>
				<ul class="flex flex-col gap-0.5">
					{#each roots.list as root (root.id)}
						{@const active = activeRoot === root.id}
						<li>
							<a
								href="/search?root={encodeURIComponent(root.id)}"
								class="flex min-h-11 items-center gap-3 rounded-lg px-3 text-[15px] {active
									? 'bg-fill font-semibold'
									: 'hover:bg-fill'}"
								aria-current={active ? 'page' : undefined}
							>
								<Icon name={root.kind === 'local' ? 'drive' : 'cloud'} size={20} class="text-accent-text" />
								<span class="min-w-0 flex-1 truncate">{root.display_name}</span>
								<RootHealthDot status={root.status} />
							</a>
						</li>
					{/each}
				</ul>
			</section>
		{/if}

		<div class="mt-auto">{@render navLink(SETTINGS)}</div>
	</div>
</nav>
