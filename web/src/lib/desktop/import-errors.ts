import type { ImportErrorCode, ImportFailure } from '../api/types';

/**
 * The page's wording for each import error code (book/src/contract.md,
 * "导入的错误码"). The import's own messages are English, so the page never
 * shows them as the headline.
 */
const TEXT: Record<ImportErrorCode, string> = {
	source_not_found: '这里没有找到旧 kikoeru 的数据，或者读不了。请选旧 kikoeru 的数据目录，也就是放着 sqlite 和 config 文件夹的那一层。',
	not_sqlite: '找到的数据库文件不是 SQLite 数据库。',
	several_sources: '这个文件夹里有好几份旧 kikoeru 的数据，请只选其中一份的数据目录。',
	guessed_paths: '有的路径是推测出来的。核对下面的路径，没问题就勾选“使用推测出来的路径”，再检查一次。',
	unmerged_journal:
		'旧数据库旁边有还没合并的日志文件，旧服务可能还在运行。最好先停掉旧服务再检查。也可以勾选“在副本上恢复日志”，旧数据不会被改动。',
	source_changed: '复制旧数据库的时候它被改动了，旧服务可能还在运行。请先停掉旧服务，再检查一次。',
	not_kikoeru: '这个数据库缺少 kikoeru 的表或列，不像是 kikoeru 的数据库。',
	too_old: '旧库来自 0.6.14 以前的版本。请先把旧服务升级到 0.6.14 或更新的版本并启动一次，再导入。',
	mysql: '旧服务用的是 MySQL，这里只能导入 SQLite 的旧库。',
	config_unreadable: '旧的 config.json 读不了。',
	moved_database: '旧配置把数据库放在了这台电脑上找不到的地方。',
	target_not_empty: '新数据目录不是空的。',
	target_overlaps: '新数据目录和旧数据重叠了。',
	no_library: '没有找到要合并进去的库。',
	would_refuse: '旧库有下面这些问题，不能导入。',
	cancelled: '导入被取消了，本机库没有改动。',
	other: '导入失败。'
};

const CODES = new Set(Object.keys(TEXT));

/** What the user can switch on to get past a code, then check again. */
export type ImportAction = 'accept_guessed' | 'recover_journal';

export function importAction(code: ImportErrorCode): ImportAction | null {
	if (code === 'guessed_paths') return 'accept_guessed';
	if (code === 'unmerged_journal') return 'recover_journal';
	return null;
}

export function importErrorText(code: ImportErrorCode): string {
	return TEXT[code];
}

/**
 * The import commands reject with an `ImportFailure`. An older shell, or a
 * failure of Tauri itself, rejects with a string, which becomes `other`.
 */
export function toImportFailure(error: unknown): ImportFailure {
	if (typeof error === 'object' && error !== null && 'code' in error && 'message' in error) {
		const { code, message } = error as { code: unknown; message: unknown };
		if (typeof code === 'string' && CODES.has(code) && typeof message === 'string') {
			return { code: code as ImportErrorCode, message };
		}
	}
	const message =
		typeof error === 'string' ? error : error instanceof Error ? error.message : '桌面应用没有响应';
	return { code: 'other', message };
}
