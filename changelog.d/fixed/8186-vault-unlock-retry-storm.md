A daemon holding a `vault.enc` it cannot unlock no longer retries the unlock on every read.
On a host with a vault file but no `LIBREFANG_VAULT_KEY` and no keyring — the exact case the vault routes answer 503 for — each `vault_get` took an exclusive lock that serialised every vault read in the daemon, ran an OS keyring lookup (DBus on Linux, potentially a Keychain prompt on macOS) and logged a warning.
The approvals status route calls it twice per request and the channel bridge once per message, so a quiet daemon produced a steady stream of both.
A reload that fails is now remembered against that exact file, and forgotten the moment the file changes, so `librefang vault init` or a rotate-key from another process still gets a real retry on the next call. (#8186) (@DaBlitzStein)
