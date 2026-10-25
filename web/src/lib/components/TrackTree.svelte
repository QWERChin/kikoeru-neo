<script lang="ts">
	import { SvelteSet } from 'svelte/reactivity';
	import type { TrackItem } from '../api/types';
	import { formatClock, formatDuration, formatSize } from '../format';
	import { isMedia } from '../player/queue';
	import type { TreeFile, TreeFolder } from '../track-tree';
	import Icon, { type IconName } from './Icon.svelte';
	import Progressive from './Progressive.svelte';

	interface Props {
		root: TreeFolder;
		/** Folders open on first render, usually the path to the resume point. */
		initiallyOpen: string[];
		currentTrackId: number | null;
		onPlay: (track: TrackItem) => void;
		/** Without these the rows have no download buttons. */
		onDownloadFile?: (track: TrackItem) => void;
		onDownloadFolder?: (folder: TreeFolder) => void;
	}

	let { root, initiallyOpen, currentTrackId, onPlay, onDownloadFile, onDownloadFolder }: Props = $props();

	// Only the initial value is read; later toggles are the user's.
	// svelte-ignore state_referenced_locally
	const open = new SvelteSet<string>(initiallyOpen);

	function toggle(path: string) {
		if (open.has(path)) open.delete(path);
		else open.add(path);
	}

	const KIND_ICONS: Record<TrackItem['kind'], IconName> = {
		audio: 'music',
		video: 'video',
		subtitle: 'captions',
		image: 'image',
		text: 'file',
		other: 'file'
	};

	/** The server answers 404 for a folder whose files are all missing. */
	function hasDownloads(folder: TreeFolder): boolean {
		return folder.files.some((f) => !f.track.missing) || folder.folders.some(hasDownloads);
	}

	function folderSummary(folder: TreeFolder): string {
		const parts = [`${folder.playableCount} 条`];
		if (folder.playableCount > 0 && folder.durationMs !== null) {
			parts.push(formatDuration(folder.durationMs));
		}
		return parts.join(' · ');
	}
</script>

{#snippet fileRow(file: TreeFile, depth: number)}
	{@const track = file.track}
	{@const current = track.id === currentTrackId}
	{@const indent = `padding-left: ${1 + depth * 1.25}rem`}
	{#if isMedia(track) && !track.missing}
		<li class="flex border-b border-separator last:border-b-0">
			<button
				type="button"
				class="flex min-h-11 min-w-0 flex-1 items-center gap-3 py-2.5 text-left hover:bg-fill {onDownloadFile
					? 'pr-1'
					: 'pr-4'} {current ? 'text-accent-text' : ''}"
				style={indent}
				aria-current={current ? 'true' : undefined}
				onclick={() => onPlay(track)}
			>
				<Icon name={current ? 'speaker' : KIND_ICONS[track.kind]} size={20} class="shrink-0 {current ? '' : 'text-accent-text'}" />
				<span class="min-w-0 flex-1 text-[15px] break-words {current ? 'font-semibold' : ''}">
					{file.name}
				</span>
				{#if track.has_subtitle}
					<span class="shrink-0 rounded border border-current px-1 text-[11px] leading-4 text-secondary">
						字幕
					</span>
				{/if}
				<span class="shrink-0 text-[13px] text-secondary tabular-nums">
					{track.duration_ms === null ? '' : formatClock(track.duration_ms / 1000)}
				</span>
			</button>
			{@render downloadButton(`下载 ${file.name}`, onDownloadFile && (() => onDownloadFile?.(track)), '')}
		</li>
	{:else}
		<!-- Missing media and non-media files are listed but cannot be played. -->
		<li
			class="flex min-h-11 items-center gap-3 border-b border-separator py-2.5 text-secondary last:border-b-0 {onDownloadFile &&
			!track.missing
				? ''
				: 'pr-4'}"
			style={indent}
		>
			<Icon name={KIND_ICONS[track.kind]} size={20} class="shrink-0" />
			<span class="min-w-0 flex-1 text-[15px] break-words" class:line-through={track.missing}>
				{file.name}
			</span>
			<span class="shrink-0 text-[13px] tabular-nums">
				{#if track.missing}
					缺失
				{:else if track.size !== null}
					{formatSize(track.size)}
				{/if}
			</span>
			{#if !track.missing}
				<!-- The row's own padding already makes it 44px tall. -->
				{@render downloadButton(`下载 ${file.name}`, onDownloadFile && (() => onDownloadFile?.(track)), '-my-2.5')}
			{/if}
		</li>
	{/if}
{/snippet}

{#snippet downloadButton(label: string, onclick: (() => void) | undefined, margin: string)}
	{#if onclick}
		<button
			type="button"
			class="grid size-11 shrink-0 place-items-center self-center text-secondary hover:text-accent-text {margin}"
			aria-label={label}
			title={label}
			{onclick}
		>
			<Icon name="download" size={18} />
		</button>
	{/if}
{/snippet}

{#snippet folderBody(folder: TreeFolder, depth: number)}
	{#each folder.folders as child (child.path)}
		{@const expanded = open.has(child.path)}
		{@const downloadFolder =
			onDownloadFolder && hasDownloads(child) ? () => onDownloadFolder?.(child) : undefined}
		<li class="border-b border-separator last:border-b-0">
			<div class="flex">
				<button
					type="button"
					class="flex min-h-11 min-w-0 flex-1 items-center gap-3 py-2.5 text-left hover:bg-fill {downloadFolder
						? 'pr-1'
						: 'pr-4'}"
					style="padding-left: {1 + depth * 1.25}rem"
					aria-expanded={expanded}
					onclick={() => toggle(child.path)}
				>
					<Icon name="folder" size={20} class="shrink-0 text-accent-text" />
					<span class="min-w-0 flex-1 text-[15px] font-medium break-words">{child.name}</span>
					<span class="shrink-0 text-[13px] text-secondary">{folderSummary(child)}</span>
					<Icon
						name="chevronRight"
						size={16}
						class="shrink-0 text-secondary transition-transform {expanded ? 'rotate-90' : ''}"
					/>
				</button>
				{@render downloadButton(`下载文件夹 ${child.name}`, downloadFolder, '')}
			</div>
			{#if expanded}
				<ul class="border-t border-separator">
					{@render folderBody(child, depth + 1)}
				</ul>
			{/if}
		</li>
	{/each}
	<Progressive items={folder.files} key={(file) => file.track.id}>
		{#snippet row(file)}
			{@render fileRow(file, depth)}
		{/snippet}
	</Progressive>
{/snippet}

<ul class="overflow-hidden rounded-xl bg-surface" aria-label="曲目">
	{@render folderBody(root, 0)}
</ul>
