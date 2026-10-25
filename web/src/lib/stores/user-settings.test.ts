import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import settingsSample from '../../../../contract/loudness/user-settings.json';
import { ApiError } from '../api/errors';
import type { UserSettings } from '../api/types';
import { DEFAULT_USER_SETTINGS } from '../player/loudness';
import { session } from './session.svelte';

const api = vi.hoisted(() => ({
	getUserSettings: vi.fn(),
	putUserSettings: vi.fn()
}));
vi.mock('../api/index', async () => ({ ...api, ApiError: (await import('../api/errors')).ApiError }));

const { userSettings } = await import('./user-settings.svelte');

const saved = settingsSample as UserSettings;
const quieter: UserSettings = { ...DEFAULT_USER_SETTINGS, normalize_loudness: true, target_lufs: -20 };
const louder: UserSettings = { ...DEFAULT_USER_SETTINGS, normalize_loudness: true, target_lufs: -12 };
const off: UserSettings = { ...DEFAULT_USER_SETTINGS };

/** A promise the test settles by hand. */
function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (error: unknown) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

const settle = async () => {
	for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 0));
};

let account = 0;
function signIn() {
	account++;
	session.signIn(`token-${account}`, { id: account, name: `user-${account}`, is_admin: false });
}

describe('userSettings', () => {
	beforeEach(() => {
		vi.clearAllMocks();
		api.putUserSettings.mockImplementation(async (value: UserSettings) => value);
		signIn();
	});

	afterEach(() => session.clear());

	it('shows the defaults before the settings load', () => {
		api.getUserSettings.mockReturnValue(new Promise(() => {}));
		void userSettings.load();
		expect(userSettings.value).toEqual(DEFAULT_USER_SETTINGS);
		expect(userSettings.error).toBeNull();
	});

	it('shows the defaults while signed out and asks for nothing', async () => {
		session.clear();
		await userSettings.load();
		await userSettings.save(quieter);
		expect(api.getUserSettings).not.toHaveBeenCalled();
		expect(api.putUserSettings).not.toHaveBeenCalled();
		expect(userSettings.value).toEqual(DEFAULT_USER_SETTINGS);
	});

	it('loads once per session', async () => {
		api.getUserSettings.mockResolvedValue(saved);
		await Promise.all([userSettings.load(), userSettings.load()]);
		await userSettings.load();
		expect(api.getUserSettings).toHaveBeenCalledTimes(1);
		expect(userSettings.value).toEqual(saved);
	});

	it('keeps a change made while the settings were loading', async () => {
		const answer = deferred<UserSettings>();
		api.getUserSettings.mockReturnValue(answer.promise);
		const loading = userSettings.load();
		void userSettings.save(quieter);
		answer.resolve(saved);
		await loading;
		await settle();
		expect(userSettings.value).toEqual(quieter);
	});

	it('sends saves one at a time, with the latest value', async () => {
		const first = deferred<UserSettings>();
		api.putUserSettings.mockReturnValueOnce(first.promise);
		void userSettings.save(quieter);
		void userSettings.save(louder);
		const last = userSettings.save(off);
		expect(userSettings.value).toEqual(off);
		expect(api.putUserSettings).toHaveBeenCalledTimes(1);
		expect(api.putUserSettings).toHaveBeenLastCalledWith(quieter);

		first.resolve(quieter);
		await last;
		// The value in between was never sent, and the first answer did not flash back.
		expect(api.putUserSettings.mock.calls).toEqual([[quieter], [off]]);
		expect(userSettings.value).toEqual(off);
	});

	it('goes back to the last confirmed value when a save fails', async () => {
		api.getUserSettings.mockResolvedValue(saved);
		await userSettings.load();
		api.putUserSettings.mockRejectedValueOnce(new ApiError(400, 'invalid_request', '目标响度超出范围'));
		await userSettings.save(quieter);
		expect(userSettings.value).toEqual(saved);
		expect(userSettings.error).toContain('目标响度超出范围');

		// The next save clears the error.
		await userSettings.save(louder);
		expect(userSettings.error).toBeNull();
		expect(userSettings.value).toEqual(louder);
	});

	it('goes back to the defaults when a save fails before anything was confirmed', async () => {
		api.putUserSettings.mockRejectedValueOnce(new Error('offline'));
		await userSettings.save(quieter);
		expect(userSettings.value).toEqual(DEFAULT_USER_SETTINGS);
		expect(userSettings.error).not.toBeNull();
	});

	it('sends a newer value instead of reverting when an older save fails', async () => {
		const first = deferred<UserSettings>();
		api.putUserSettings.mockReturnValueOnce(first.promise);
		void userSettings.save(quieter);
		const last = userSettings.save(louder);
		first.reject(new Error('offline'));
		await last;
		expect(api.putUserSettings.mock.calls).toEqual([[quieter], [louder]]);
		expect(userSettings.value).toEqual(louder);
		expect(userSettings.error).toBeNull();
	});

	it('discards a load answer for the previous session', async () => {
		const answer = deferred<UserSettings>();
		api.getUserSettings.mockReturnValueOnce(answer.promise);
		const loading = userSettings.load();
		signIn();
		answer.resolve(saved);
		await loading;
		expect(userSettings.value).toEqual(DEFAULT_USER_SETTINGS);

		// The new session asks for its own settings.
		api.getUserSettings.mockResolvedValueOnce(off);
		await userSettings.load();
		expect(api.getUserSettings).toHaveBeenCalledTimes(2);
		expect(userSettings.value).toEqual(off);
	});

	it('discards a save answer or failure for the previous session', async () => {
		const answer = deferred<UserSettings>();
		api.putUserSettings.mockReturnValueOnce(answer.promise);
		const saving = userSettings.save(quieter);
		signIn();
		expect(userSettings.value).toEqual(DEFAULT_USER_SETTINGS);
		answer.reject(new ApiError(500, 'internal_error', '服务器出错了'));
		await saving;
		expect(userSettings.value).toEqual(DEFAULT_USER_SETTINGS);
		expect(userSettings.error).toBeNull();
	});

	it('saves a change made in a new session while the old session save is still out', async () => {
		const answer = deferred<UserSettings>();
		api.putUserSettings.mockReturnValueOnce(answer.promise);
		void userSettings.save(quieter);
		signIn();
		const saving = userSettings.save(louder);
		answer.resolve(quieter);
		await saving;
		await settle();
		expect(api.putUserSettings.mock.calls).toEqual([[quieter], [louder]]);
	});

	it('keeps a save that succeeded before the first load answered', async () => {
		const answer = deferred<UserSettings>();
		api.getUserSettings.mockReturnValueOnce(answer.promise);
		const loading = userSettings.load();
		await userSettings.save(quieter);
		answer.resolve(off);
		await loading;
		expect(userSettings.value).toEqual(quieter);
		// A failed save goes back to what the server last confirmed, the save.
		api.putUserSettings.mockRejectedValueOnce(new ApiError(500, 'internal_error', '服务器出错了'));
		await userSettings.save(louder);
		expect(userSettings.value).toEqual(quieter);
	});

	it('clears the load error once a retry succeeds', async () => {
		api.getUserSettings.mockRejectedValueOnce(new ApiError(500, 'internal_error', '服务器出错了'));
		await userSettings.load();
		expect(userSettings.error).toContain('播放设置没有读到');
		api.getUserSettings.mockResolvedValueOnce(saved);
		await userSettings.load();
		expect(userSettings.error).toBeNull();
		expect(userSettings.value).toEqual(saved);
	});
});
