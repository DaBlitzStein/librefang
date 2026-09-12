Logging in to the dashboard now survives a daemon restart.
`sessions.json` was written on every login, logout and cleanup sweep and then discarded in full every time it was read: the file keys each row by a one-way `$sha256$` digest while the daemon looked sessions up by the cleartext token, so no restored row could ever match one.
Restarting the service logged every operator out, which is not how any other service behaves.
The lookup now hashes the token the caller presents, so a restored row matches without the daemon ever holding the cleartext, and the file on disk is byte-for-byte the format it was before — still only digests, still with the token field cleared.
Nothing an attacker could do with a stolen `sessions.json` changes; only the daemon's willingness to read its own file does.
One operational note, because it cuts against the headline: a session is also now evaluated against the permission gate even when its stored row carries no role, which it previously skipped entirely.
An operator carrying a `sessions.json` old enough to predate role attribution will find that session restored but refused on every write until they log in again.
(#8268) (@DaBlitzStein)
