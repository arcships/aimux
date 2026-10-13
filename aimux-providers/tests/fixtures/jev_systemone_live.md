# Official TypeSafe live recording

`jev_systemone_live.jsonl` contains two calls recorded on 2026-10-06 by aimux's
unified `JsonlRecorder` through the Python binding. The official endpoint was
`https://api.typesafe.ai/v1/systemone`; requested model `jev-latest` returned
`jev-1.13.0`.

The first call uses plain text state/descriptions. The second uses native JSON
state, object/array instructions, Boolean criteria, a structured Choice
description and structured Score levels. Each exercises Boolean, Choice and
Score. Inputs are synthetic test data. Authorization is automatically redacted.

These replace the earlier manually collected JSON fixture. Each recording
includes the actual HTTP exchange, timing and the result `decide` returned.
Offline provider tests replay the wire exchange via the shared replay helper;
Core and the CLI replay the recorded result.

Stack integration migrates the recording envelope to schema 3: provider
identity only, and no capability snapshot. The decision options and results
were then converted, offline and deterministically, to the DecisionModelV4
shape: the options are `decide` options (a question map with criteria), and
the result is `decide`'s (answers, usage with its total, warnings, rounding,
provider metadata with the probability source and per-answer confidence, and
the response with the recording time as its timestamp). The HTTP exchanges,
including the request and response bodies, are unchanged.

The wire-format layer uses camelCase for the recording envelope and shared Usage/Timeout types. The original HTTP request and response body strings remain unchanged.
