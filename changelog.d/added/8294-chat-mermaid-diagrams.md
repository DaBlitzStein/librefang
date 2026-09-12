Mermaid diagrams in a chat message are now drawn instead of shown as their source.
Agents answer with Mermaid often — an architecture sketch, a sequence diagram for a flow they just traced — and until now you read `graph TD; A-->B;` and had to picture it yourself.
A diagram that does not render still gives you its source back rather than an error, and the source stays one click away under every diagram that does.
Diagrams are fitted to the chat column rather than allowed to set its width — mermaid writes its own width onto the SVG, and a source asking for a fixed pixel width would otherwise decide how wide the conversation is — and clicking one opens it full size in a view that scrolls both ways.
Nothing is downloaded until a diagram actually appears: the library and its layout engine are split out of the main bundle, and a fence only becomes a diagram once the turn has settled, so a half-written one reads as text while it streams. (#8294) (@DaBlitzStein)
