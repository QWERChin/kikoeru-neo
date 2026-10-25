<script lang="ts">
	import { onDestroy } from 'svelte';
	import { ApiError, putMetadata } from '../../api/index';
	import { metadataSaves } from '../../stores/metadata-saves.svelte';
	import type { WorkDetail } from '../../api/types';
	import { formFromDetail, toMetadataRequest, type MetadataField, type MetadataForm } from '../../metadata-form';
	import TitleBarDrag from '../TitleBarDrag.svelte';

	interface Props {
		work: WorkDetail;
		open: boolean;
	}

	let { work, open = $bindable() }: Props = $props();

	const uid = $props.id();
	let dialog = $state<HTMLDialogElement | null>(null);
	let form = $state<MetadataForm>({ title: '', releaseDate: '', nsfw: false, circle: '', vas: '', tags: '' });
	let errors = $state<Partial<Record<MetadataField, string>>>({});
	let saveError = $state<string | null>(null);
	let saving = $derived(metadataSaves.isSaving(work.id));
	// Counts openings, so a save from an earlier opening cannot close or
	// unlock a dialog that was reopened while it was pending.
	let opening = 0;
	// A destroyed editor still holds the setter of `bind:open`, which may now
	// belong to the editor of another work, so a late answer must not use it.
	let destroyed = false;
	onDestroy(() => (destroyed = true));

	$effect(() => {
		if (!dialog) return;
		if (open && !dialog.open) {
			// Every opening starts from the work as it is now.
			opening++;
			form = formFromDetail(work);
			errors = {};
			saveError = null;
			dialog.showModal();
		} else if (!open && dialog.open) {
			dialog.close();
		}
	});

	async function save(event: SubmitEvent) {
		event.preventDefault();
		if (saving) return;
		const result = toMetadataRequest(form);
		if (!result.ok) {
			errors = result.errors;
			return;
		}
		errors = {};
		saveError = null;
		const current = opening;
		const id = work.id;
		try {
			// One save per work at a time, across openings and page visits. The
			// detail page reloads through metadataSaves when a save succeeds.
			const detail = await metadataSaves.run(id, () => putMetadata(id, result.request));
			if (detail === null) return;
			if (current === opening && !destroyed) open = false;
		} catch (e) {
			if (current === opening && !destroyed) saveError = `保存失败：${e instanceof ApiError ? e.message : '未知错误'}`;
		}
	}

	const field = 'min-h-11 w-full rounded-lg bg-fill px-3 py-2 text-[17px] text-text';
	const labelText = 'text-[13px] font-medium text-secondary';
</script>

<dialog
	bind:this={dialog}
	class="m-auto max-h-[calc(100dvh-2rem-env(safe-area-inset-top)-env(safe-area-inset-bottom)-2*var(--title-bar-h))] w-[min(32rem,calc(100vw-2rem))] flex-col overflow-hidden rounded-2xl bg-surface-raised p-0 text-text shadow-xl backdrop:bg-black/40 open:flex"
	aria-labelledby="{uid}-title"
	onclose={() => (open = false)}
>
	<TitleBarDrag />
	<!-- Header and buttons stay put while the fields scroll, so 保存 is always on screen. -->
	<form class="flex min-h-0 flex-auto flex-col" novalidate onsubmit={save}>
		<h2 id="{uid}-title" class="shrink-0 border-b border-separator px-5 py-4 text-[20px] font-semibold">编辑元数据</h2>
		<!-- The fields are locked while saving: the dialog closes when the save
		     succeeds, and anything typed after the request went out would be lost.
		     A fieldset would do it in one place but does not scroll in a flex column. -->
		<div class="flex min-h-0 flex-auto flex-col gap-4 overflow-y-auto px-5 py-4">
		<p class="rounded-lg bg-fill px-3 py-2 text-[13px] text-secondary">
			保存后，这部作品的元数据标记为手动修改。之后普通的“重新抓取”不会覆盖它，只有选了“强制覆盖”的重新抓取才会。
		</p>

		<label class="flex flex-col gap-1">
			<span class={labelText}>标题</span>
			<input
				class={field}
				bind:value={form.title}
				disabled={saving}
				oninput={() => (errors = { ...errors, title: undefined })}
				required
				aria-invalid={errors.title ? 'true' : undefined}
				aria-describedby={errors.title ? `${uid}-title-error` : undefined}
			/>
			{#if errors.title}
				<span id="{uid}-title-error" class="text-[13px] text-danger">{errors.title}</span>
			{/if}
		</label>

		<label class="flex flex-col gap-1">
			<span class={labelText}>社团</span>
			<input class={field} bind:value={form.circle} disabled={saving} placeholder="不填表示未知社团" />
		</label>

		<label class="flex flex-col gap-1">
			<span class={labelText}>声优</span>
			<textarea class="{field} min-h-20" bind:value={form.vas} disabled={saving} rows="3" aria-describedby="{uid}-names"></textarea>
		</label>

		<label class="flex flex-col gap-1">
			<span class={labelText}>标签</span>
			<textarea class="{field} min-h-20" bind:value={form.tags} disabled={saving} rows="3" aria-describedby="{uid}-names"></textarea>
		</label>
		<p id="{uid}-names" class="-mt-2 text-[13px] text-secondary">
			每行写一个名字，名字里可以有逗号。没有的社团、声优和标签会新建，标签会先按别名归到已有的规范标签。
		</p>

		<label class="flex flex-col gap-1">
			<span class={labelText}>发售日</span>
			<input
				class={field}
				type="date"
				bind:value={form.releaseDate}
				disabled={saving}
				oninput={() => (errors = { ...errors, releaseDate: undefined })}
				aria-invalid={errors.releaseDate ? 'true' : undefined}
				aria-describedby={errors.releaseDate ? `${uid}-date-error` : undefined}
			/>
			{#if errors.releaseDate}
				<span id="{uid}-date-error" class="text-[13px] text-danger">{errors.releaseDate}</span>
			{/if}
		</label>

		<label class="flex min-h-11 items-center gap-3">
			<input type="checkbox" class="size-5 accent-[var(--accent)]" bind:checked={form.nsfw} disabled={saving} />
			<span>R18 作品</span>
		</label>

		{#if saveError}
			<p class="text-[15px] text-danger" role="alert">{saveError}</p>
		{/if}
		</div>

		<div class="flex shrink-0 justify-end gap-2 border-t border-separator px-5 py-3">
			<button type="button" class="min-h-11 rounded-lg px-4 text-accent-text" onclick={() => (open = false)}>
				取消
			</button>
			<button
				type="submit"
				class="min-h-11 rounded-lg bg-accent px-5 font-semibold text-on-accent disabled:opacity-50"
				disabled={saving}
			>
				{saving ? '正在保存' : '保存'}
			</button>
		</div>
	</form>
</dialog>
