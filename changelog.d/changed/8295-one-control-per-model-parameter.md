Every model parameter now uses one control everywhere it can be set.
The agent editor offered preset rungs for the context window, the model settings a slider running from 1 Ki to 2 Mi in steps of 1024 — on which landing exactly on 131072 was a matter of pixels — and the per-provider override a bare number box, each with its own word for "leave this alone".
Temperature, top-p and the two penalties had the same split: a 0.01-step slider with a separate on/off switch in the model settings, four bare number boxes in the agent editor.
All of them are now the same object, which owns the presets, the accepted range and the wording, so they cannot drift apart again.
The rungs are discrete because the choices are: a handful of named behaviours for a sampling parameter, an order-of-magnitude sequence for a token count, and a value the endpoint will refuse is now refused by the control rather than saved and sent. (#8295) (@DaBlitzStein)
