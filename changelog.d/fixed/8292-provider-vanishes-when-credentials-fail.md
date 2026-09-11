A provider no longer disappears from the Providers page when its credentials stop working.
Saving a key the endpoint rejected, or a local service going down, left the entry in a state the page read as "not configured" and removed it from view entirely — so the one provider that needed attention was the one you could no longer see, recoverable only by finding it again in the Add picker.
It now stays where it was, flagged, with its configure action one click away.
Providers you have suppressed move the other way: out of the Add picker by default, behind a toggle that brings them back.
The Providers search box and every API-key field also carry explicit autofill hints now, because Firefox was reading the pair as a login form and filling the key field with a saved password, which then overwrote the real credential on save. (#8292) (@DaBlitzStein)
