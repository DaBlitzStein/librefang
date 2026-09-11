The context window, output cap and max-tokens fields now use one control everywhere they can be set.
The agent editor offered preset rungs, the model settings a slider running from 1 Ki to 2 Mi in steps of 1024 — on which landing exactly on 131072 was a matter of pixels — and the per-provider override a bare number box, each with its own word for "leave this alone".
All three are now the same object, which owns the presets and the wording, so they cannot drift apart again.
Sliders stay where a slider is right: temperature, top-p and the penalties are continuous, token counts are not. (#8295) (@DaBlitzStein)
