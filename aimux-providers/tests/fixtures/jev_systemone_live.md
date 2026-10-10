# Official TypeSafe live recording

`jev_systemone_live.jsonl` contains two calls recorded on 2026-10-06 by aimux's
unified `JsonlRecorder` through the Python binding. The official endpoint was
`https://api.typesafe.ai/v1/systemone`; requested model `jev-latest` returned
`jev-1.13.0`.

The first call uses plain text state/descriptions. The second uses native JSON
state, object/array instructions, Boolean criteria, a structured Choice
description and structured Score levels. Each exercises Boolean, Choice and
Score. Inputs are synthetic test data. Authorization is automatically redacted.

These replace the earlier manually collected JSON fixture. Standard Recording
The recording includes the actual HTTP exchange, timing, capability snapshot and
normalized result. Offline provider tests replay the wire exchange via the
shared replay helper; Core and the CLI replay the normalized result.

Stack integration migrates the recording envelope to schema 3: provider identity only, with decision capabilities under input. Recorded requests, responses and normalized outcomes are unchanged.

The wire-format layer uses camelCase for the recording envelope and shared Usage/Timeout types. The original HTTP request and response body strings remain unchanged.
