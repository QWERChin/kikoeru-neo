<script lang="ts">
	import { page } from '$app/state';
</script>

<main class="mx-auto max-w-md px-4 py-16">
	<h1 class="text-[34px] leading-[41px] font-bold tracking-tight">{page.status}</h1>
	<p class="mt-2 text-secondary">{page.error?.message ?? '页面出错了'}</p>
	<a href="/library" class="mt-6 inline-flex min-h-11 items-center text-accent-text">回到资料库</a>
</main>
