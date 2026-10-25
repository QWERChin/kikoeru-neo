<script lang="ts">
	import { ApiError, checkProxy, getServerSettings, putServerSettings } from '#lib/api/index.ts';
	import { desktop } from '#lib/desktop/desktop.svelte.ts';
	import { proxyCheckText, proxyEdited, proxyErrorText, proxySavedText, proxySettings } from '#lib/metadata-proxy.ts';

	const uid = $props.id();
	const reason = (e: unknown) => (e instanceof ApiError ? e.message : '未知错误');

	let loaded = $state(false);
	let loadError = $state<string | null>(null);
	let saved = $state<string | null>(null);
	let input = $state('');
	let saving = $state(false);
	let checking = $state(false);
	let checked = $state<string | null>(null);
	let checkError = $state<string | null>(null);
	let savedNote = $state<string | null>(null);
	let saveError = $state<string | null>(null);

	let edited = $derived(proxyEdited(input, saved));

	$effect(() => {
		getServerSettings().then(
			(settings) => {
				saved = settings.metadata_proxy;
				input = saved ?? '';
				loaded = true;
			},
			(e) => (loadError = `代理设置加载失败：${reason(e)}`)
		);
	});

	// The notes describe the value that was tested or saved, not the one being typed.
	function clearNotes() {
		checked = checkError = savedNote = saveError = null;
	}

	async function check() {
		checking = true;
		clearNotes();
		const tested = input;
		try {
			const text = proxyCheckText(await checkProxy(proxySettings(tested)));
			// A result for a value the admin has since changed would vouch for the wrong proxy.
			if (input === tested) checked = text;
		} catch (e) {
			if (input === tested) checkError = proxyErrorText(e, 'check');
		} finally {
			checking = false;
		}
	}

	async function save(event: SubmitEvent) {
		event.preventDefault();
		if (saving || !edited) return;
		saving = true;
		clearNotes();
		const sent = input;
		try {
			saved = (await putServerSettings(proxySettings(sent))).metadata_proxy;
			// Once the admin has typed something else, the field is not what was
			// saved, so it keeps the new text and gets no note about the old one.
			if (input === sent) {
				input = saved ?? '';
				savedNote = proxySavedText(saved);
			}
		} catch (e) {
			if (input === sent) saveError = proxyErrorText(e, 'save');
		} finally {
			saving = false;
		}
	}
</script>

<section aria-labelledby="settings-proxy">
	<h2 id="settings-proxy" class="px-1 pb-1 text-[13px] font-semibold tracking-wide text-secondary">网络代理</h2>
	{#if loadError}
		<p class="rounded-xl bg-surface px-4 py-3 text-[15px] text-danger" role="alert">{loadError}</p>
	{:else if !loaded}
		<div class="h-24 animate-pulse rounded-xl bg-surface" aria-busy="true" aria-label="正在加载"></div>
	{:else}
		<form class="flex flex-col gap-3 rounded-xl bg-surface px-4 py-3" onsubmit={save} novalidate>
			<label class="flex flex-col gap-1 text-[13px] text-secondary">
				元数据代理
				<input
					class="min-h-11 rounded-lg bg-fill px-3 text-[15px] text-text"
					type="text"
					inputmode="url"
					autocomplete="off"
					spellcheck="false"
					placeholder="http://127.0.0.1:7890"
					aria-describedby="{uid}-help"
					bind:value={input}
					oninput={clearNotes}
				/>
			</label>
			<div id="{uid}-help" class="flex flex-col gap-1 text-[13px] text-secondary">
				<p>
					抓取 DLsite、asmr.one 和 hvdb 的元数据和封面时走这个代理，媒体库不走。留空时按服务器的环境变量 HTTPS_PROXY
					决定，没有就直连。支持 http、https、socks5 和 socks5h，不支持带账号密码的代理。
				</p>
				{#if desktop.kind === 'local'}
					<p>桌面版不会自动使用系统代理，需要走代理时请在这里填写。</p>
				{/if}
			</div>
			<div>
				<div class="flex flex-wrap gap-2">
					<button
						type="button"
						class="min-h-11 rounded-lg bg-fill px-4 text-[15px] text-accent-text disabled:opacity-50"
						disabled={checking}
						onclick={() => void check()}
					>
						{checking ? '测试中…' : '测试连接'}
					</button>
					<button
						type="submit"
						class="min-h-11 rounded-lg bg-accent px-4 font-semibold text-on-accent disabled:opacity-50"
						disabled={saving || !edited}
					>
						保存
					</button>
				</div>
				<!-- Always in the page, so screen readers notice text appearing in it. -->
				<div aria-live="polite">
					{#if checked}<p class="mt-3 text-[15px]">{checked}</p>{/if}
					{#if savedNote}<p class="mt-3 text-[15px]">{savedNote}</p>{/if}
				</div>
				{#if checkError}<p class="mt-3 text-[15px] text-danger" role="alert">{checkError}</p>{/if}
				{#if saveError}<p class="mt-3 text-[15px] text-danger" role="alert">{saveError}</p>{/if}
			</div>
		</form>
	{/if}
</section>
