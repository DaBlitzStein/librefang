A vault write no longer loses a concurrent write from a different process.
`CredentialVault::save` re-encrypts the whole file from one instance's in-memory map, so two writers that each read before either wrote leave only the second one's entry, and the first disappears with no error raised anywhere — over HTTP, after the route has already answered 200.
The kernel's in-process lock could never cover this: `KernelOAuthProvider` opens its own instance for the `mcp-oauth:*` entries and `librefang vault set` runs in a separate process entirely.
Nor was the window narrow, since each read and each save pays an Argon2id at m=19456 KiB, t=2.
`set`, `remove` and `rewrap_with_new_key` now take a cross-process advisory lock on `vault.enc.lock` and re-read the file inside it, which also covers the one-time sentinel backfill on vaults predating the startup-validation check.
Rotation was the worst of the three to lose a write to, because afterwards the old key no longer opens the file and the dropped entry cannot be read back by retrying with it. (#8186) (@DaBlitzStein)
