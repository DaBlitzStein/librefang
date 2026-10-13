A signed-in user can now set their own emoji and avatar image from the dashboard, in the user form's edit drawer.
The daemon could already store both, and nothing in the web UI mentioned either, so the user's side of a chat bubble fell back to a generic icon and the user list drew the name alone, whatever the identity said.
The editor is drawn for the caller's own row and no other, and that falls out of the two reads underneath it rather than being a policy choice.
The only user-avatar path the dashboard may fetch is the literal `/api/users/me/avatar`, because a user name does not fit the authenticated-image allowlist's character class — `encodeURIComponent("Juan Pérez")` carries a `%`, and widening the class to accept `%XX` would readmit `%2F`, which decodes to the `/` that allowlist exists to stop — and `UserItem` is the one user-shaped type the dashboard declares without `emoji` on it, so there is no list-shaped read to seed a draft from.
Over another operator's row the editor would have drawn the signed-in user's own picture beside that user's name and seeded the field from the signed-in user's glyph. (#8387) (@DaBlitzStein)
Let an Owner set their own emoji and avatar image from the dashboard, in the edit drawer of their own row on the users page, and draw it on the user's side of every chat message.
The daemon could already store both, but nothing in the web UI set or showed either, so every user message carried the same generic icon.
A chat bubble now draws the signed-in user's picture, then their emoji, then their initials, and keeps the generic icon for the synthetic `root` of a no-auth or master-key session, which names no user.
The editor appears only on the caller's own row because the dashboard can read only the caller's own emoji and picture, through `whoami` and `/api/users/me/avatar`, and only for an Owner because every write under `/api/users` is Owner-only on the daemon.
The users list itself is unchanged (#8387) (@DaBlitzStein)
