// Texts and request bodies for the metadata proxy section of the settings page.
import { ApiError, CLIENT_ERROR_CODES } from './api/errors';
import type { ProxyCheckResult, ServerSettings } from './api/types';

/** The request body for what the admin typed. A blank field means no proxy, as on the server. */
export function proxySettings(input: string): ServerSettings {
	const value = input.trim();
	return { metadata_proxy: value === '' ? null : value };
}

/** Whether saving the field would change the stored setting. */
export function proxyEdited(input: string, saved: string | null): boolean {
	return input.trim() !== (saved ?? '');
}

/**
 * Any HTTP answer means the proxy reached DLsite. A 4xx or 5xx usually comes
 * from the proxy or DLsite refusing the request, which is not the same as no
 * connection, so it gets its own text (book/src/contract.md, "服务器设置").
 */
export function proxyCheckText({ status, elapsed_ms }: ProxyCheckResult): string {
	if (status >= 200 && status < 400) return `连接正常：DLsite 回答了 HTTP ${status}，用时 ${elapsed_ms} 毫秒。`;
	return `连上了 DLsite，但它回答了 HTTP ${status}，可能是代理或 DLsite 那边的限制。`;
}

export function proxySavedText(saved: string | null): string {
	return saved === null ? '已清除，之后按环境变量决定。' : '已保存，之后的元数据请求会走这个代理。';
}

/**
 * A failed check or save. The server's 502 and 503 for a check already say
 * that DLsite could not be reached or that there is no metadata source, so
 * they are shown as they are.
 */
export function proxyErrorText(error: unknown, action: 'check' | 'save'): string {
	const prefix = action === 'check' ? '测试没有完成：' : '没有保存：';
	if (!(error instanceof ApiError)) return `${prefix}未知错误`;
	if (error.code === CLIENT_ERROR_CODES.network) return `${prefix}连不上服务器，请检查网络后重试。`;
	if (action === 'check' && error.code === 'upstream_error') return error.message;
	return `${prefix}${error.message}`;
}
