// What the library sheet says for each error of the roots API
// (book/src/contract.md, "媒体库管理接口"). The server's message is only
// shown where it carries the detail, such as why OpenList failed.
import { ApiError, CLIENT_ERROR_CODES } from '../api/errors';

export type RootAction = 'create' | 'update' | 'delete';

/** 502 and 503 from a save: the OpenList check failed, and `skip_check` can save anyway. */
export function isCheckFailure(error: unknown): error is ApiError {
	return (
		error instanceof ApiError &&
		(error.code === 'storage_unavailable' || error.code === 'upstream_error') &&
		(error.status === 502 || error.status === 503)
	);
}

export function isScanRunning(error: unknown): boolean {
	return error instanceof ApiError && error.code === 'scan_running';
}

const VERB: Record<RootAction, string> = { create: '添加', update: '保存', delete: '删除' };

export function rootErrorText(error: unknown, action: RootAction, id = ''): string {
	const verb = VERB[action];
	if (!(error instanceof ApiError)) return `${verb}失败：未知错误`;
	switch (error.code) {
		case 'root_exists':
			return `已经有 ID 为“${id}”的媒体库，它可能写在配置文件里。请在“高级”里换一个 ID。`;
		case 'root_read_only':
			return '这个媒体库写在服务器的配置文件里，只能在 config.toml 里修改，改完重启服务。';
		case 'scan_running':
			return action === 'delete'
				? '这个媒体库正在扫描或排队扫描，现在不能删除。可以在“扫描进度”里取消扫描，或者等扫描结束后再删除。'
				: '这个媒体库正在扫描，扫描时不能修改路径、地址、目录或用户名。请在“扫描进度”里取消扫描，或者等扫描结束后再保存。';
		case 'storage_unavailable':
		case 'upstream_error':
			if (action === 'delete') return `${verb}失败：${error.message}`;
			return `连接检查没有通过：${error.message}`;
		case 'invalid_request':
			return `设置不正确：${error.message}`;
		case 'not_found':
			return '这个媒体库已经不存在了，可能刚被删除。列表已经刷新。';
		case 'forbidden':
			return '只有管理员可以管理媒体库。';
		case CLIENT_ERROR_CODES.network:
			return `${verb}失败：连不上服务器，请检查网络后重试。`;
		default:
			return `${verb}失败：${error.message}`;
	}
}
