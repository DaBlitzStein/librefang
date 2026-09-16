A test now fails the build if any CLI locale file defines the same Fluent message key twice.
The `union` git merge driver concatenates text instead of detecting conflicts, so two branches that each add a key to the same `.ftl` file merge cleanly and leave both copies behind.
Fluent rejects the whole resource for that language when that happens: a duplicate in the default English pack panics `i18n::init` and takes down every test that touches it, while a duplicate in another pack fails silently and falls back to English for that entire language.
Both failure modes have already happened once each, discovered only after the fact (#8353) (@DaBlitzStein)
