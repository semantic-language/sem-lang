# sem-lang editor example

This workspace demonstrates the complete editor path without external access:

1. `work/instruction.sem` contains only the user's instruction in Japanese. Product
   policy and analyzer explanations belong here or in typed project metadata,
   not in every semantic source document.
2. `project.sem` selects the exact semantic definition revision.
3. Each `.sem-lang/services/<meaning>/capability.json` declares one exact Zixcel
   interaction resource. Its directory path is the resource identity; the
   example contains personal/work calendars, an organization meeting space,
   and an external appointment directory.
4. Each
   `.sem-lang/services/authentication/<provider>/<account>/authentication.json` selects
   one credential-free authentication reference. It never stores tokens,
   secrets, or account credentials.
5. Each mirrored `.sem-lang/runtime/services/**/state.json` is a non-secret
   runtime-state projection. This offline reference workspace keeps two so both
   ready and authorization-required rendering can be tested.
6. The language server reports unresolved meaning or unavailable capabilities;
   it never invents either.
7. The VS Code view keeps every route for the same reservation intent visible.
   It shows route selection, exact missing fields, candidate destinations, and
   authentication state next to the semantic graph.

This repository includes the reference workspace, not a published VS Code
extension or VSIX download. Inspect its source and contracts directly. A separately
provided compatible editor integration may render the semantic graph; do not
assume that integration is installed or publicly distributed.

Before opening a fresh checkout, explicitly initialize its ignored durable
graph once:

```text
sem-lang-cli project init-storage --project project.sem
```

The editor and compiler intentionally do not create missing storage while
reading or saving source.

The current editor example performs definition and interaction-contract
inspection. It does not resolve relative civil time, choose an execution route
or calendar, obtain credentials, or execute a reservation. Treat the example as
source/reference inspection, not a working booking service. Definition revisions,
capability identity, non-secret authentication references and current runtime
state remain explicit inputs. Missing inputs stay unresolved rather than being
inferred. See the [public introduction](../../docs/getting-started.md) for the
language and local-memory boundary.
