import { describe, expect, it } from 'vitest';
import { importAction, importErrorText, toImportFailure } from './import-errors';

describe('import errors', () => {
	it('keeps a coded failure from the shell', () => {
		expect(toImportFailure({ code: 'too_old', message: 'older than 0.6.14' })).toEqual({
			code: 'too_old',
			message: 'older than 0.6.14'
		});
	});

	it('turns a plain string or an unknown code into other', () => {
		expect(toImportFailure('对话框打不开')).toEqual({ code: 'other', message: '对话框打不开' });
		expect(toImportFailure({ code: 'brand_new', message: 'x' })).toEqual({
			code: 'other',
			message: '桌面应用没有响应'
		});
	});

	it('words every code in Chinese and offers the two actions', () => {
		expect(importErrorText('too_old')).toContain('0.6.14');
		expect(importErrorText('guessed_paths')).toContain('使用推测出来的路径');
		expect(importAction('guessed_paths')).toBe('accept_guessed');
		expect(importAction('unmerged_journal')).toBe('recover_journal');
		expect(importAction('source_changed')).toBeNull();
	});
});
