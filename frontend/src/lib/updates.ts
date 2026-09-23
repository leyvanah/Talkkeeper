/**
 * Whether the app may look for updates at all.
 *
 * Off until this project publishes signed releases of its own. The updater
 * this code inherited pointed at the original project's releases and trusted
 * that project's signing key: a newer version there would have been offered —
 * and installed — over this one, and it cannot read an encrypted archive.
 *
 * Turning this back on needs four things together: releases on this
 * project's repository, the updater and process plugins registered again in
 * the backend (removed so that no key but this project's can be trusted), a
 * signing key of its own in tauri.conf.json, and that repository's
 * latest.json as the updater endpoint.
 */
export const UPDATES_AVAILABLE = false;
