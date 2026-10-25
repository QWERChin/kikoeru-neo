/**
 * Decides when a tab reloads for a new service worker. Accepting the update
 * in one tab activates the worker for every tab, so a tab only reloads after
 * its own user pressed 刷新. The other tabs keep playing on the old code.
 */
export class UpdateConsent {
	#accepted = false;
	#reloading = false;

	/**
	 * The user pressed 刷新. Returns true when the tab should reload right
	 * away, because another tab already activated this worker and no
	 * `controllerchange` will follow.
	 */
	accept(alreadyActive: boolean): boolean {
		this.#accepted = true;
		return alreadyActive && this.shouldReload();
	}

	/** Called on `controllerchange`; true once, and only for an accepting tab. */
	shouldReload(): boolean {
		if (!this.#accepted || this.#reloading) return false;
		this.#reloading = true;
		return true;
	}
}
