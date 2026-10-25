import type { DesktopState } from '../api/types';
import {
	inShell,
	shellCommands,
	staticPageKind,
	tauriIpc,
	type PageLocation,
	type ShellCommands,
	type ShellPage
} from './shell';

export interface DesktopDeps {
	inShell: () => boolean;
	location: () => PageLocation | null;
	commands: ShellCommands;
}

/** Which shell page this is, and what the shell last said about its mode. */
export class Desktop {
	/** `null` only while a page in the shell waits for `desktop_state`. */
	kind = $state<ShellPage | null>(null);
	state = $state<DesktopState | null>(null);

	readonly commands: ShellCommands;
	#detecting: Promise<ShellPage> | null = null;

	constructor(deps: DesktopDeps) {
		this.commands = deps.commands;
		const location = deps.location();
		// Known at once in a browser and in the picker, so neither ever
		// renders a frame of the other kind's UI.
		this.kind = location ? staticPageKind(deps.inShell(), location) : 'browser';
		if (this.kind) this.#detecting = Promise.resolve(this.kind);
	}

	/**
	 * Only local pages may call `desktop_state` outside the picker, so its
	 * answer is what tells a local page from a remote one. The mode it
	 * reports can lag behind while the shell is still switching.
	 */
	detect(): Promise<ShellPage> {
		this.#detecting ??= this.commands.desktopState().then(
			(state) => {
				this.state = state;
				this.kind = 'local';
				return 'local' as const;
			},
			() => {
				this.kind = 'remote';
				return 'remote' as const;
			}
		);
		return this.#detecting;
	}
}

export const desktop = new Desktop({
	inShell,
	location: () => (typeof location === 'undefined' ? null : location),
	commands: shellCommands(tauriIpc)
});
