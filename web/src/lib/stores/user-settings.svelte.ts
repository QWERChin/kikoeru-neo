import { ApiError, getUserSettings, putUserSettings } from '../api/index';
import type { UserSettings } from '../api/types';
import { DEFAULT_USER_SETTINGS } from '../player/loudness';
import { session } from './session.svelte';

interface Keyed {
	key: string;
	value: UserSettings;
}

/**
 * The signed-in user's `/api/me/settings`. Until they load, and when they
 * cannot, the defaults apply, so normalisation stays off. A change shows
 * at once and is saved in the background. Saves go out one at a time with
 * the latest value, because each PUT replaces the whole object and two in
 * flight could arrive in either order.
 */
class UserSettingsStore {
	#shown = $state<Keyed | null>(null);
	/** What the server last confirmed, which a failed save goes back to. */
	#confirmed: Keyed | null = null;
	#error = $state<{ key: string; message: string; from: 'load' | 'save' } | null>(null);
	#loading: { key: string; promise: Promise<void> } | null = null;
	#pending: Keyed | null = null;
	#saving: Promise<void> | null = null;

	/** The signed-in user's settings arrived (or were changed here) in this session. */
	get known(): boolean {
		return session.key !== null && this.#shown?.key === session.key;
	}

	get value(): UserSettings {
		return this.#shown?.key === session.key ? this.#shown.value : DEFAULT_USER_SETTINGS;
	}

	get error(): string | null {
		return this.#error?.key === session.key ? this.#error.message : null;
	}

	/** Loads once per session. A change made meanwhile wins over the answer. */
	load(): Promise<void> {
		const key = session.key;
		if (key === null) return Promise.resolve();
		if (this.#loading?.key === key) return this.#loading.promise;
		if (this.#confirmed?.key === key) return Promise.resolve();
		const promise = getUserSettings()
			.then((value) => {
				if (session.key !== key) return;
				// A save that succeeded while this GET was out is newer than its answer.
				if (this.#confirmed?.key !== key) this.#confirmed = { key, value };
				if (this.#shown?.key !== key) this.#shown = { key, value };
				if (this.#error?.key === key && this.#error.from === 'load') this.#error = null;
			})
			.catch((e: unknown) => {
				if (session.key !== key) return;
				this.#error = { key, message: `播放设置没有读到：${message(e)}`, from: 'load' };
			})
			.finally(() => {
				// After a failure the settings page asks again when it opens.
				if (this.#loading?.promise === promise) this.#loading = null;
			});
		this.#loading = { key, promise };
		return promise;
	}

	save(next: UserSettings): Promise<void> {
		const key = session.key;
		if (key === null) return Promise.resolve();
		this.#shown = { key, value: next };
		this.#error = null;
		this.#pending = { key, value: next };
		this.#saving ??= this.#drain().finally(() => (this.#saving = null));
		return this.#saving;
	}

	// A flush for an account that has signed out stops after its PUT, so a
	// change the next account made meanwhile is flushed here instead of lost.
	async #drain(): Promise<void> {
		while (this.#pending) {
			if (this.#pending.key !== session.key) {
				this.#pending = null;
				return;
			}
			await this.#flush(this.#pending.key);
		}
	}

	async #flush(key: string): Promise<void> {
		while (this.#pending?.key === key && session.key === key) {
			const value = this.#pending.value;
			this.#pending = null;
			try {
				const saved = await putUserSettings(value);
				if (session.key !== key) return;
				this.#confirmed = { key, value: saved };
				if (!this.#pending) this.#shown = { key, value: saved };
			} catch (e) {
				if (session.key !== key) return;
				if (this.#pending) continue;
				this.#shown = this.#confirmed?.key === key ? this.#confirmed : null;
				this.#error = { key, message: `没有保存：${message(e)}`, from: 'save' };
			}
		}
	}
}

function message(e: unknown): string {
	return e instanceof ApiError ? e.message : '未知错误';
}

export const userSettings = new UserSettingsStore();
