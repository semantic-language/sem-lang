# sem-lang-language

Resolves Linguistic Knowledge into generated resources and immutable CompiledLanguage artifacts.

Typed definitions are keyed by `TermId`, never by display label. A duplicate
label cannot uniquely resolve a term; declared lexical bindings may still
identify distinct meanings. Competing meanings for the same surface and
grammatical contribution return `AmbiguousSurface` with candidate IDs, not a
priority-selected guess. Distinct grammatical contributions can compose.

The compiled digest includes typed module inputs as well as generated resources;
changing a semantic definition or binding constraint invalidates it even when
labels do not change. This digest does not establish publisher trust or grant
permission to execute an application operation.
