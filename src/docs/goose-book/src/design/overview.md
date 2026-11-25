# Design Investigations

Goose is pre-1.0, which leaves room to investigate a design before building
it: survey how other tools solve the same problem, draft a design, put it
through adversarial review, prototype the reviewed design behind an
off-by-default Cargo feature, and measure it before deciding whether it
belongs in a release.

This section collects that work. Each page states its decision on the
first line — build it, build a smaller version of it, defer it, or don't
build it — and the evidence behind that decision.

- [Request Hooks](request-hooks.md): whether Goose should get a request
  hook (middleware) layer for signing requests, adding trace headers,
  response checks, token pull-through, traffic recording and fault
  injection.
