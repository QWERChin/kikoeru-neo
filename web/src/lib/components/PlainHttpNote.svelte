<script lang="ts">
	import { isInsecureContext } from '../secure-context';

	// The app is a client-only SPA, so the browser's answer is there at once.
	const insecure = isInsecureContext();
</script>

{#if insecure}
	<section class="rounded-xl bg-surface px-4 py-3" role="note" aria-label="通过 HTTP 访问">
		<p>通过 HTTP 访问</p>
		<p class="text-[13px] text-secondary">
			登录、浏览、播放和设置都能正常使用。浏览器只在 HTTPS 下提供 Service
			Worker，所以现在不能安装成应用，也不能离线打开页面。需要这两项时，请改用 HTTPS 地址访问。
		</p>
	</section>
{/if}
