import { coverCookieWrites } from '../api/cover-cookie';
import type { User } from '../api/types';

const TOKEN_KEY = 'kikoeru-neo.token';
const MEDIA_TOKEN_KEY = 'kikoeru-neo.media-token';

// Storage throws in private windows and when site data is blocked. The
// session then lives in memory only, which still works until a reload.
function readStorage(key: string): string | null {
	try {
		return localStorage.getItem(key);
	} catch {
		return null;
	}
}

function writeStorage(key: string, value: string | null): boolean {
	try {
		if (value === null) localStorage.removeItem(key);
		else localStorage.setItem(key, value);
		return true;
	} catch {
		// Memory still holds the value; see readStorage.
		return false;
	}
}

function writeCoverCookie(token: string | null): void {
	if (typeof document === 'undefined') return;
	for (const write of coverCookieWrites(token, location.protocol === 'https:')) {
		document.cookie = write;
	}
}

class Session {
	token = $state<string | null>(null);
	mediaToken = $state<string | null>(null);
	user = $state<User | null>(null);
	private tokenBeforeFailedWrite: string | null = null;
	/**
	 * Bumped when the account may have changed: on sign-in and sign-out,
	 * but not when the desktop shell renews the token of the same account.
	 */
	account = $state(0);

	/**
	 * Identifies the signed-in account for caches and late answers. Unlike
	 * the token, it stays the same when the shell renews the token.
	 */
	get key(): string | null {
		return this.token === null ? null : `account-${this.account}`;
	}

	constructor() {
		if (typeof window !== 'undefined') {
			this.token = readStorage(TOKEN_KEY);
			this.mediaToken = readStorage(MEDIA_TOKEN_KEY);
			// The cookie may be missing or older than the stored token, for
			// instance after an update from a version without it.
			if (this.mediaToken !== null) writeCoverCookie(this.mediaToken);
		}
	}

	signIn(token: string, user: User): void {
		this.token = token;
		this.user = user;
		this.account++;
		const previousToken = readStorage(TOKEN_KEY);
		this.tokenBeforeFailedWrite = writeStorage(TOKEN_KEY, token) ? null : previousToken;
	}

	/** The token in storage, which the desktop shell replaces when it renews the session. */
	storedToken(): string | null {
		return readStorage(TOKEN_KEY);
	}

	/** Takes a newer token of the same account, which is already in storage. */
	renewToken(token: string): void {
		this.token = token;
		this.tokenBeforeFailedWrite = null;
	}

	setMediaToken(token: string | null): void {
		this.mediaToken = token;
		writeStorage(MEDIA_TOKEN_KEY, token);
		writeCoverCookie(token);
	}

	clear(): void {
		// A failed sign-in write can leave the previous token in storage.
		// Only a token different from both values belongs to another tab.
		const storedToken = readStorage(TOKEN_KEY);
		if (
			storedToken === null || storedToken === this.token || storedToken === this.tokenBeforeFailedWrite
		) {
			writeStorage(TOKEN_KEY, null);
			writeStorage(MEDIA_TOKEN_KEY, null);
			writeCoverCookie(null);
		}
		this.token = null;
		this.mediaToken = null;
		this.user = null;
		this.tokenBeforeFailedWrite = null;
		this.account++;
	}
}

export const session = new Session();
