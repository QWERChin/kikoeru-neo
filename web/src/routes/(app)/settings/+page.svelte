<script lang="ts" module>
	import type { StartedScan } from '#lib/tasks/root-scan.ts';

	// Scans started from this page, kept while the app is open so a scan
	// still has its library's name after leaving the page and coming back.
	const startedScans = new Map<number, StartedScan>();
</script>

<script lang="ts">
	import { goto } from '$app/navigation';
	import { onDestroy, untrack } from 'svelte';
	import { ApiError, cancelTask, listTasks, logoutAll, moveTaskToFront, session, startScan, taskWatchDeps } from '#lib/api/index.ts';
	import type { RootStatus, TaskSnapshot } from '#lib/api/types/index.ts';
	import MetadataProxySection from '#lib/components/admin/MetadataProxySection.svelte';
	import MissingSection from '#lib/components/admin/MissingSection.svelte';
	import RootSheet from '#lib/components/admin/RootSheet.svelte';
	import TagMergeSection from '#lib/components/admin/TagMergeSection.svelte';
	import TaskList from '#lib/components/admin/TaskList.svelte';
	import TaskProgressCard from '#lib/components/admin/TaskProgressCard.svelte';
	import DesktopSettings from '#lib/components/desktop/DesktopSettings.svelte';
	import LegacyImport from '#lib/components/desktop/LegacyImport.svelte';
	import Icon from '#lib/components/Icon.svelte';
	import LargeTitle from '#lib/components/LargeTitle.svelte';
	import PlainHttpNote from '#lib/components/PlainHttpNote.svelte';
	import AppearanceSettings from '#lib/components/AppearanceSettings.svelte';
	import ChangePassword from '#lib/components/ChangePassword.svelte';
	import PlaybackSettings from '#lib/components/PlaybackSettings.svelte';
	import RootHealthDot from '#lib/components/RootHealthDot.svelte';
	import { LiveTasks } from '#lib/stores/live-tasks.svelte.ts';
	import { roots } from '#lib/stores/roots.svelte.ts';
	import { TaskActivity } from '#lib/stores/task-activity.svelte.ts';
	import { desktop } from '#lib/desktop/desktop.svelte.ts';
	import { inRunOrder } from '#lib/tasks/queue-order.ts';
	import { rootScanText, scansOfRoot } from '#lib/tasks/root-scan.ts';
	import { watchTask } from '#lib/tasks/task-watch.ts';

	const KIND_LABELS = { local: '本地', openlist: 'OpenList', webdav: 'WebDAV' } as const;
	const HEALTH_LABELS = { ok: '正常', unavailable: '无法访问', unknown: '还没检查' } as const;

	// Bumped when a followed task ends, so the lists that it changes reload.
	let finishedCount = $state(0);

	function onFinished(snapshot: TaskSnapshot) {
		finishedCount++;
		if (snapshot.kind === 'scan') void roots.reload();
		activity.poke();
	}

	const live = new LiveTasks(
		{
			watch: (id, options) => watchTask(id, options, taskWatchDeps),
			cancel: cancelTask,
			moveToFront: moveTaskToFront
		},
		onFinished
	);
	const activity = new TaskActivity({ listTasks });

	// The settings page is where an admin checks a library, so it always
	// asks for fresh status instead of reusing the sidebar's copy.
	$effect(() => {
		if (!roots.visible) return;
		void roots.load(true);
		activity.start();
		return () => activity.stop();
	});

	// Scans that were running before the page opened, or were started
	// elsewhere, are followed as soon as the activity poll sees them.
	$effect(() => {
		const scans = activity.scans;
		// Untracked: following writes the live list, which must not rerun this.
		untrack(() => {
			for (const scan of scans) {
				live.follow(scan.id, startedScans.get(scan.id)?.label ?? `扫描任务 #${scan.id}`, scan);
			}
		});
	});

	// Leaving the page only drops the streams. The tasks keep running.
	onDestroy(() => live.closeAll());

	let fullScan = $state(false);
	let scanning = $state(false);
	let scanError = $state<string | null>(null);

	function rootScans(root: RootStatus) {
		return scansOfRoot(root.id, roots.list?.length ?? 0, live.items, startedScans);
	}

	async function scan(root: RootStatus | null) {
		scanning = true;
		scanError = null;
		const full = fullScan;
		try {
			const { task_ids } = await startScan({ root: root?.id ?? null, ...(full && { full: true }) });
			const suffix = full ? ' · 完整扫描' : '';
			task_ids.forEach((id, i) => {
				// "Scan all" answers one task per library, in an order the
				// contract does not fix, so those tasks get a generic name.
				const name = root
					? root.display_name
					: task_ids.length === 1
						? '全部媒体库'
						: `全部媒体库 ${i + 1}/${task_ids.length}`;
				const label = `${name}${suffix}`;
				// A repeated request joins the running scan, which keeps its name.
				if (!startedScans.has(id)) startedScans.set(id, { label, root: root?.id ?? null });
				live.follow(id, startedScans.get(id)!.label);
			});
			activity.poke();
		} catch (e) {
			scanError = `扫描没有开始：${e instanceof ApiError ? e.message : '未知错误'}`;
		} finally {
			scanning = false;
		}
	}

	let sheetOpen = $state(false);
	let sheetRoot = $state<RootStatus | null>(null);

	function openSheet(root: RootStatus | null) {
		sheetRoot = root;
		sheetOpen = true;
	}

	function rootsChanged() {
		void roots.reload();
		// A deleted root's works show up in the missing list.
		finishedCount++;
	}

	const dateFormat = new Intl.DateTimeFormat('zh-CN', { dateStyle: 'medium', timeStyle: 'short' });

	function signOut() {
		session.clear();
		goto('/login', { replaceState: true });
	}

	let confirmingLogoutAll = $state(false);
	let loggingOutAll = $state(false);
	let logoutAllError = $state<string | null>(null);

	async function logOutEverywhere() {
		loggingOutAll = true;
		logoutAllError = null;
		// A slow answer can arrive after the user signed out and in as someone
		// else, whose session it must not touch.
		const account = session.account;
		try {
			await logoutAll();
			// The token this page holds is revoked too, so it signs out here as well.
			if (session.account === account) signOut();
		} catch (e) {
			if (session.account === account) {
				logoutAllError = `没有退出：${e instanceof ApiError ? e.message : '未知错误'}`;
			}
		} finally {
			loggingOutAll = false;
		}
	}
</script>

<svelte:head><title>设置 · kikoeru</title></svelte:head>

{#snippet addRoot()}
	<button
		type="button"
		class="mt-2 flex min-h-11 w-full items-center gap-3 rounded-xl bg-surface px-4 py-3 text-left text-accent-text hover:bg-fill"
		onclick={() => openSheet(null)}
	>
		<Icon name="plus" size={20} />
		添加媒体库
	</button>
{/snippet}

<LargeTitle title="设置" />

<!-- Wide windows get two columns, so the page does not leave half the screen empty.
     Narrower ones show one column. There the column wrappers drop out of the
     layout, so 外观 can come first instead of after the admin's library groups.
     grid-cols-1 sizes that column minmax(0, 1fr), so a long library name
     truncates instead of widening the page, as the wrappers' min-w-0 did. -->
<div class="grid grid-cols-1 gap-6 px-4 md:max-w-2xl md:px-8 xl:max-w-6xl xl:grid-cols-2 xl:items-start">
	{#if roots.visible}
	<div class="contents xl:flex xl:min-w-0 xl:flex-col xl:gap-6">
		<section aria-labelledby="settings-roots">
			<div class="flex items-center justify-between px-1 pb-1">
				<h2 id="settings-roots" class="text-[13px] font-semibold tracking-wide text-secondary">媒体库</h2>
				<button
					type="button"
					class="min-h-11 px-2 text-[15px] text-accent-text"
					onclick={() => void roots.load(true)}
				>
					刷新
				</button>
			</div>
			{#if roots.stale && roots.error}
				<p class="mb-2 rounded-xl bg-surface px-4 py-3 text-[15px] text-danger" role="alert">
					刷新失败：{roots.error.message}。下面是上次取到的状态，可能已经过时。
				</p>
			{/if}
			{#if roots.error && !roots.list}
				<p class="rounded-xl bg-surface px-4 py-3 text-[15px] text-danger" role="alert">
					媒体库状态加载失败：{roots.error.message}
				</p>
			{:else if !roots.list}
				<div class="h-24 animate-pulse rounded-xl bg-surface" aria-busy="true" aria-label="正在加载"></div>
			{:else if roots.list.length === 0}
				<p class="rounded-xl bg-surface px-4 py-3 text-[15px] text-secondary">还没有媒体库。添加一个文件夹或 OpenList 网盘之后就可以扫描。</p>
				{@render addRoot()}
			{:else}
				<ul class="overflow-hidden rounded-xl bg-surface" aria-label="媒体库状态">
					{#each roots.list as root (root.id)}
						{@const scanText = rootScanText(rootScans(root))}
						<li class="flex min-h-11 items-center gap-3 border-b border-separator px-4 py-3 last:border-b-0">
							<Icon name={root.kind === 'local' ? 'drive' : 'cloud'} size={20} class="shrink-0 text-accent-text" />
							<div class="min-w-0 flex-1">
								<p class="flex min-w-0 items-center gap-2">
									<span class="truncate">{root.display_name}</span>
									{#if root.source === 'config'}
										<span class="shrink-0 rounded-full bg-fill px-2 py-0.5 text-[11px] text-secondary">由配置文件管理</span>
									{/if}
								</p>
								<p
									class="flex flex-wrap items-center gap-x-1.5 text-[13px] {root.status === 'unavailable'
										? 'text-danger'
										: 'text-secondary'}"
								>
									<span class="text-secondary">{KIND_LABELS[root.kind]} · {root.work_count} 部 ·</span>
									<RootHealthDot status={root.status} />
									{HEALTH_LABELS[root.status]}
								</p>
								<!-- A running scan replaces the last scan time, which it is about to change. -->
								<p class="text-[13px] tabular-nums {scanText ? 'text-accent-text' : 'text-secondary'}">
									{scanText ??
										(root.last_scan_at === null ? '还没扫描过' : `上次扫描 ${dateFormat.format(root.last_scan_at)}`)}
								</p>
								{#if root.watching || root.next_scan_at !== null}
									<p class="flex flex-wrap items-center gap-x-1.5 text-[13px] text-secondary tabular-nums">
										{#if root.watching}
											<span class="inline-flex items-center gap-1.5">
												<span class="size-2 rounded-full bg-listening" aria-hidden="true"></span>正在监视
											</span>
										{/if}
										{#if root.watching && root.next_scan_at !== null}<span aria-hidden="true">·</span>{/if}
										{#if root.next_scan_at !== null}
											<span>下次自动扫描 {dateFormat.format(root.next_scan_at)}</span>
										{/if}
									</p>
								{/if}
							</div>
							<button
								type="button"
								class="flex size-11 shrink-0 items-center justify-center rounded-lg text-accent-text"
								aria-label="{root.source === 'config' ? '查看' : '编辑'} {root.display_name}"
								onclick={() => openSheet(root)}
							>
								<Icon name="info" size={22} />
							</button>
							<button
								type="button"
								class="min-h-11 shrink-0 rounded-lg bg-fill px-3 text-[15px] text-accent-text disabled:opacity-50"
								aria-label="扫描 {root.display_name}"
								disabled={scanning}
								onclick={() => void scan(root)}
							>
								{scanText ? '扫描中' : '扫描'}
							</button>
						</li>
					{/each}
				</ul>
				{@render addRoot()}
				{#if roots.list.some((r) => r.status === 'unavailable')}
					<p class="px-1 pt-2 text-[13px] text-secondary">
						无法访问的媒体库里的作品、收藏和播放进度都还保留着，恢复连接之后就能继续播放。
					</p>
				{/if}
				<div class="mt-2 flex flex-col gap-2 rounded-xl bg-surface px-4 py-3">
					<label class="flex min-h-11 items-center gap-3">
						<input type="checkbox" class="size-5 shrink-0 accent-[var(--accent)]" bind:checked={fullScan} />
						<span>完整扫描（刷新网盘缓存）</span>
					</label>
					<p class="text-[13px] text-secondary">
						OpenList 媒体库会重新遍历每部作品，不再跳过目录没变的作品，耗时更长，请求也更多。本地媒体库不受影响。
					</p>
					<button
						type="button"
						class="min-h-11 self-start rounded-lg bg-accent px-4 font-semibold text-on-accent disabled:opacity-50"
						disabled={scanning}
						onclick={() => void scan(null)}
					>
						{fullScan ? '完整扫描全部' : '扫描全部'}
					</button>
					{#if scanError}
						<p class="text-[15px] text-danger" role="alert">{scanError}</p>
					{/if}
				</div>
			{/if}
		</section>

		<section aria-labelledby="settings-progress">
			<h2 id="settings-progress" class="px-1 pb-1 text-[13px] font-semibold tracking-wide text-secondary">
				扫描进度
			</h2>
			{#if live.items.length === 0}
				<p class="rounded-xl bg-surface px-4 py-3 text-[15px] text-secondary">现在没有正在跟踪的扫描。</p>
			{:else}
				<ul class="overflow-hidden rounded-xl bg-surface" aria-label="扫描进度">
					{#each inRunOrder(live.items) as task (task.id)}
						<TaskProgressCard
							{task}
							onCancel={() => void live.cancel(task.id)}
							onDismiss={() => live.dismiss(task.id)}
							onMoveToFront={() => void live.moveToFront(task.id)}
						/>
					{/each}
				</ul>
				<p class="px-1 pt-2 text-[13px] text-secondary">
					扫描一次只跑一个，排队的按从上到下的顺序开始。离开这个页面只会停止显示进度，扫描会继续运行。要停下扫描，请点“取消任务”。
				</p>
			{/if}
			<div class="mt-2 flex min-h-11 items-center gap-3 rounded-xl bg-surface px-4 py-3" role="group" aria-label="后台探测">
				<div class="min-w-0 flex-1">
					<p>后台探测</p>
					<p class="text-[13px] text-secondary">本地音频的时长探测在扫描之外单独排队，扫描完成后可能还在进行，不影响播放。</p>
				</div>
				<p class="shrink-0 text-right text-[13px] text-secondary" aria-live="polite">
					{#if !activity.probes}
						读取中
					{:else if activity.probes.queued + activity.probes.running === 0}
						没有进行中的探测
					{:else}
						{activity.probes.running} 个进行中<br />{activity.probes.queued}{activity.probes.capped ? '+' : ''} 个排队
					{/if}
				</p>
			</div>
		</section>

		<MetadataProxySection />

		<TagMergeSection version={finishedCount} />
	</div>
	<RootSheet
		bind:open={sheetOpen}
		root={sheetRoot}
		takenIds={roots.list?.map((r) => r.id) ?? []}
		onChanged={rootsChanged}
	/>
	{/if}

	<div class="contents xl:flex xl:min-w-0 xl:flex-col xl:gap-6">
	<div class="min-w-0 max-xl:order-first">
		<AppearanceSettings />
	</div>
	<DesktopSettings />
	<PlaybackSettings />
	<!-- The import merges into this computer's library, so only its admin gets it. -->
	{#if desktop.kind === 'local' && roots.visible}
		<LegacyImport />
	{/if}
	{#if roots.visible}
		<TaskList version={finishedCount} />
		<MissingSection version={finishedCount} />
	{/if}

	<PlainHttpNote />

	<section class="overflow-hidden rounded-xl bg-surface" aria-label="账号">
		{#if session.user}
			<div class="flex min-h-11 items-center border-b border-separator px-4 py-3 last:border-b-0">
				<span class="flex-1">当前用户</span>
				<span class="text-secondary">{session.user.name}</span>
			</div>
		{/if}
		<a
			href="/reviews"
			class="flex min-h-11 items-center gap-2 border-b border-separator px-4 py-3 last:border-b-0 hover:bg-fill"
		>
			<Icon name="star" size={20} class="text-accent-text" />
			<span class="flex-1">我的评价</span>
			<Icon name="chevronRight" size={18} class="text-secondary" />
		</a>
		<!-- The shell signs the local page in, and its password stays in the keychain. -->
		{#if desktop.kind !== 'local'}
		<ChangePassword />
		<button
			type="button"
			class="flex min-h-11 w-full items-center gap-2 px-4 py-3 text-left text-danger hover:bg-fill"
			onclick={signOut}
		>
			<Icon name="logout" size={20} />
			退出登录
		</button>
		{#if confirmingLogoutAll}
			<div class="flex flex-col gap-2 border-t border-separator px-4 py-3" role="group" aria-label="退出所有设备">
				<span>所有设备上的登录都会失效，包括这台，之后要重新输入密码。</span>
				<div class="flex gap-2">
					<button
						type="button"
						class="min-h-11 flex-1 rounded-lg bg-fill px-3 disabled:opacity-50"
						disabled={loggingOutAll}
						onclick={() => (confirmingLogoutAll = false)}
					>
						取消
					</button>
					<button
						type="button"
						class="min-h-11 flex-1 rounded-lg bg-danger px-3 font-semibold text-white disabled:opacity-50"
						disabled={loggingOutAll}
						onclick={() => void logOutEverywhere()}
					>
						全部退出
					</button>
				</div>
			</div>
		{:else}
			<button
				type="button"
				class="flex min-h-11 w-full items-center gap-2 border-t border-separator px-4 py-3 text-left text-danger hover:bg-fill"
				onclick={() => (confirmingLogoutAll = true)}
			>
				<Icon name="logout" size={20} />
				退出所有设备
			</button>
		{/if}
		{/if}
	</section>
	{#if logoutAllError}
		<p class="px-1 text-[15px] text-danger" role="alert">{logoutAllError}</p>
	{/if}
	</div>
</div>
