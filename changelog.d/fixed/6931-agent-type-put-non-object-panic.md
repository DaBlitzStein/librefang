`PUT /api/templates/{name}` (and its `/api/agent-types/{name}` alias) no longer panics the daemon on a non-object JSON body.
`Json<serde_json::Value>` happily deserializes an array, string, number, or bool, but `serde_json::Value`'s `IndexMut<&str>` only handles `Null` and `Object` — every other variant panics via `panic!("cannot access key ... in ...")` on the `body["name"] = ...` write that pins the manifest name to the URL path segment.
Any caller could trip this on an existing agent type with a one-line `PUT` body of `[]` or `42`, no authentication bypass required.
The handler now rejects a non-object body with 400 before touching it (#6931) (@DaBlitzStein)
