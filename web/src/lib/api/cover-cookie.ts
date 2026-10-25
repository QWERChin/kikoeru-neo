/**
 * The cookies the server reads a cover's media token from. HTTPS and plain
 * HTTP on the same host get one each: a `Secure` cookie written over HTTPS
 * cannot be replaced from HTTP, so a shared name would leave a plain-HTTP
 * page unable to store its token.
 */
export const COVER_COOKIE = 'kikoeru-media';
export const SECURE_COVER_COOKIE = '__Secure-kikoeru-media';

/**
 * The `document.cookie` assignment that stores `token` for covers, or
 * deletes the cookie for `null`. Cover URLs then carry no token, so they
 * stay the same when the token is renewed and the browser keeps the covers
 * it cached. Strict same-site and the `/api/works` path keep the cookie off
 * every other request.
 */
export function coverCookie(token: string | null, secure: boolean): string {
	const name = secure ? SECURE_COVER_COOKIE : COVER_COOKIE;
	const attributes = `; Path=/api/works; SameSite=Strict${secure ? '; Secure' : ''}`;
	return token === null ? `${name}=${attributes}; Max-Age=0` : `${name}=${token}${attributes}`;
}

/**
 * The `document.cookie` assignments for storing `token`, or for signing out
 * with `null`. Signing out over HTTPS deletes the plain-HTTP cookie too: the
 * browser also sends that one over HTTPS, so a valid token left in it from
 * an earlier HTTP visit would keep covers loading after sign-out. Plain HTTP
 * cannot touch the `Secure` cookie, but the browser never sends it there.
 */
export function coverCookieWrites(token: string | null, secure: boolean): string[] {
	if (token !== null) return [coverCookie(token, secure)];
	return secure ? [coverCookie(null, true), coverCookie(null, false)] : [coverCookie(null, false)];
}

/**
 * The token a cover URL carries. Normally none, and the cookie authenticates
 * it. With cookies blocked, and on the one retry after a failed load, the
 * URL carries the token as it did before the cookie: the cookie may be
 * missing or unusable for reasons a token refresh cannot fix, and a new
 * token also makes a new URL, which the `<img>` loads again.
 */
export function coverToken(
	mediaToken: string | null,
	retry: boolean,
	cookiesEnabled: boolean
): string | null {
	return retry || !cookiesEnabled ? mediaToken : null;
}
