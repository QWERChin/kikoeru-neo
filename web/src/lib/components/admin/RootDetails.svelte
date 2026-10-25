<script lang="ts">
	import type { RootStatus } from '../../api/types';
	import { formatInterval } from '../../roots/form';

	// A root from config.toml, which the API only shows. The page explains
	// where to change it instead of offering fields that would answer 409.
	let { root }: { root: RootStatus } = $props();

	let rows = $derived.by((): [string, string][] => {
		const s = root.settings;
		const common: [string, string][] = [
			['名称', root.display_name],
			['ID', root.id]
		];
		const interval: [string, string] = [
			'自动扫描间隔',
			s.auto_scan_interval === null ? '关闭' : `每 ${formatInterval(s.auto_scan_interval)}`
		];
		if (s.type === 'local') {
			const watch = s.watch ? (root.watching ? '开启，正在监视' : '开启，但现在没有在监视') : '关闭';
			return [...common, ['类型', '本地文件夹'], ['路径', s.path], interval, ['监视文件夹变化', watch]];
		}
		const remote: [string, string][] = [
			...common,
			['类型', s.type === 'openlist' ? 'OpenList' : 'WebDAV'],
			['地址', s.url],
			['目录', s.base_path],
			['用户名', s.username],
			['密码', s.password_set ? '已设置' : '没有设置'],
			['同时列目录的请求数', s.list_concurrency === null ? '默认' : String(s.list_concurrency)]
		];
		if (s.type === 'openlist') remote.push(['每秒取直链的次数', s.link_rate === null ? '默认' : String(s.link_rate)]);
		return [...remote, interval];
	});
</script>

<p class="rounded-lg bg-fill px-3 py-2 text-[13px] text-secondary">
	这个媒体库写在服务器的 <code>config.toml</code> 里，这里只能查看。要修改或删除，请编辑配置文件里 <code>id = "{root.id}"</code> 的那一段
	<code>[[roots]]</code>，然后重启服务。
</p>
<dl class="overflow-hidden rounded-xl bg-surface">
	{#each rows as [label, value] (label)}
		<div class="flex min-h-11 items-center gap-3 border-b border-separator px-4 py-2 last:border-b-0">
			<dt class="shrink-0 text-secondary">{label}</dt>
			<dd class="min-w-0 flex-1 text-right break-all">{value}</dd>
		</div>
	{/each}
</dl>
