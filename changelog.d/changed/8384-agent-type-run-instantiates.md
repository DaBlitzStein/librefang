Run on an agent type now instantiates it, instead of asking which existing agent to fork.
The control sits on a type and offers a play affordance, yet it opened a modal whose first field was a dropdown of the agents that already exist, so the operator answered a question they had not asked and then re-picked the same type from a different dropdown on a different page.
It now opens the Agents page's create drawer on the template tab with that type already selected, carried in a `template` search param that is cleared when the drawer closes so a second press re-opens it.
The ephemeral-run modal goes with it, because that button was its only entry point.
(#8384) (@DaBlitzStein)
