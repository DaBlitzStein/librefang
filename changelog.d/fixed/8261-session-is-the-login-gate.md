The dashboard login can no longer be walked around by entering at `/`.
With a dashboard password configured, `/dashboard/agents` answered the login page while `/` answered the full SPA shell, because the root was on the unconditional public-route allowlist and never reached the gate at all — so whether you met the login screen depended on which URL you happened to type, not on whether you had signed in.
Both paths serve the same `index.html`, and the root was also excluded from the session-cookie lookup, so a session established there could never have been recognised anyway.
The root now follows exactly the same rule as the rest of the shell: served when no dashboard password is configured, answered with the login page when one is.
An `api_key`-only deployment is deliberately unchanged — it has no username-and-password login screen to show, and closing the shell there would leave the operator with nowhere to enter the key.
(#8261) (@DaBlitzStein)
