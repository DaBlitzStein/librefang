Switching agents while a chat reply was still coming left that chat claiming to be busy forever, and three minutes later quietly ran the same turn a second time.
Tearing down the WebSocket abandoned the turn instead of finishing it, so nothing ever re-enabled the input, and the retry armed for a genuine network drop fired against a message the daemon had already accepted — billing the work twice and writing two runs into one session's history.
Switching agents now detaches the turn rather than retrying or cancelling it: the input comes back immediately, the run keeps going, and its result arrives with the session history.
(#8258) (@DaBlitzStein)
