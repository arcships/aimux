# Legacy C export replacement ledger

All 130 old implementations are removed in the atomic C ABI switch. Four shared
names are reimplemented solely in `src/abi.rs`; 126 retired names are absent.
The final ABI exports 14 primitives, including `aimux_handle_finalizer`.

The table below maps every removed symbol to its replacement and covering test.
Business DTO generation remains in `scripts/gen_ts_types.py`; recording schema
migration is separate. Decision coverage is provided by `decision.decide` /
`decision.capabilities`.

| Old symbol | Replacement op / primitive | Replacement test target |
|---|---|---|
| `aimux_error_free` | Same-name RFC §4.1 primitive, implemented only in abi.rs | abi_primitives; tests/c/primitives.c and tests/c/raii.cpp |
| `aimux_error_code` | Same-name RFC §4.1 primitive, implemented only in abi.rs | abi_primitives; tests/c/primitives.c and tests/c/raii.cpp |
| `aimux_error_message` | Same-name RFC §4.1 primitive, implemented only in abi.rs | abi_primitives; tests/c/primitives.c and tests/c/raii.cpp |
| `aimux_error_status` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_retry_ms` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_retryable` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_provider_code` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_provider_message` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_response_body` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_url` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_request_body_values` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_response_headers` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_provider_data` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_retry_reason` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_retry_count` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_retry_error_at` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_model_id` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_model_type` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_provider_id` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_tool_name` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_available_tools` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_tool_input` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_error_original_error` | aimux_error_json: ErrorEnvelope.error / retryAfterMs; no field getter aliases | error_envelope_golden; abi_primitives |
| `aimux_openai_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_openai_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_anthropic_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_anthropic_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_anthropic_aws_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_anthropic_aws_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_azure_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_azure_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_bedrock_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_bedrock_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_vertex_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_vertex_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_cohere_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_cohere_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_mistral_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_mistral_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_xai_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_xai_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_provider_new` | provider.create / provider.default → provider.model; one-shot construction removed | ops/provider.rs tests; ops_matrix |
| `aimux_provider_from_env` | provider.create / provider.default → provider.model; one-shot construction removed | ops/provider.rs tests; ops_matrix |
| `aimux_provider_handle_new` | provider.create | ops/provider.rs tests; handle_lifecycle |
| `aimux_provider_list_models` | provider.invoke (existing model discovery only) | ops/provider.rs tests; ops_matrix |
| `aimux_provider_model` | provider.model | ops/provider.rs tests; handle_lifecycle |
| `aimux_get_model_specs` | catalogue.fetch | ops/misc.rs tests |
| `aimux_generate_text` | text.generate | ops/text.rs tests; ops_matrix |
| `aimux_generate_object` | text.generateObject | ops/text.rs tests; ops_matrix |
| `aimux_consume_stream_text` | text.consume | ops/text.rs tests; ops_matrix |
| `aimux_stream_text` | text.stream via aimux_stream; optional Abort handle | ops/text.rs tests; stream_terminal; abi_primitives |
| `aimux_abort_signal_new` | abort.create | ops/handles.rs tests; abi_primitives |
| `aimux_abort_signal_abort` | abort.signal / aimux_abort | ops/handles.rs tests; ABI callback tests |
| `aimux_abort_signal_drop` | handle.drop / aimux_drop; pointer-shaped alias removed | ops/handles.rs tests; handle_lifecycle; C RAII tests |
| `aimux_stream_text_with_abort` | text.stream via aimux_stream; optional Abort handle | ops/text.rs tests; stream_terminal; abi_primitives |
| `aimux_tool_call_repair_context` | toolRepair.context | ops/misc.rs tests |
| `aimux_apply_tool_call_repair` | toolRepair.apply | ops/misc.rs tests |
| `aimux_apply_tool_call_repair_to_result` | toolRepair.applyResult | ops/misc.rs tests |
| `aimux_generate_text_result_as_openai` | output.toOpenAI | ops/text.rs tests; ops_matrix |
| `aimux_generate_text_as_openai` | text.generateOpenAI | ops/text.rs tests; ops_matrix |
| `aimux_stream_text_as_openai` | text.streamOpenAI via aimux_stream; optional Abort handle | ops/text.rs tests; stream_terminal; abi_primitives |
| `aimux_stream_text_as_openai_with_abort` | text.streamOpenAI via aimux_stream; optional Abort handle | ops/text.rs tests; stream_terminal; abi_primitives |
| `aimux_drop_handle` | handle.drop / aimux_drop; pointer-shaped alias removed | ops/handles.rs tests; handle_lifecycle; C RAII tests |
| `aimux_drop_handle_ptr` | handle.drop / aimux_drop; pointer-shaped alias removed | ops/handles.rs tests; handle_lifecycle; C RAII tests |
| `aimux_free_string` | Same-name RFC §4.1 primitive, implemented only in abi.rs | abi_primitives; tests/c/primitives.c and tests/c/raii.cpp |
| `aimux_openai_embedding_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_openai_embedding_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_cohere_embedding_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_cohere_embedding_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_google_embedding_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_google_embedding_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_embed` | embedding.embed / embedding.embedMany | ops/modalities.rs tests; ops_matrix |
| `aimux_openai_speech_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_openai_speech_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_speech_generate` | speech.generate | ops/modalities.rs tests; ops_matrix |
| `aimux_openai_image_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_openai_image_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_google_image_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_google_image_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_image_generate` | image.generate | ops/modalities.rs tests; ops_matrix |
| `aimux_openai_transcription_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_openai_transcription_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_transcription_generate` | transcription.generate | ops/modalities.rs tests; ops_matrix |
| `aimux_transcription_session_new` | transcription.start → Operation; no legacy session type | operation integration tests; abi_primitives |
| `aimux_transcription_push_audio` | operation.push / aimux_operation_push | operation integration tests; C pointer/length tests |
| `aimux_transcription_input_done` | operation.inputDone | operation admission/close tests |
| `aimux_transcription_next_part` | operation.next / aimux_operation_next_part | operation poll/single-consumer tests; abi_primitives |
| `aimux_transcription_session_drop` | handle.drop / aimux_drop on Operation | operation cleanup tests; handle_lifecycle |
| `aimux_openai_files_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_openai_files_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_file_upload` | files.upload | ops/files.rs tests |
| `aimux_cohere_reranking_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_cohere_reranking_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_rerank` | reranking.rerank | ops/modalities.rs tests; ops_matrix |
| `aimux_google_video_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_google_video_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_video_generate` | video.generate | ops/modalities.rs tests; ops_matrix |
| `aimux_provider_decision_model` | provider.model(method=decision) | Existing core/tests/decision_test.rs and providers/tests/runtime_decision_test.rs retained; ops/decision.rs tests, ops_matrix |
| `aimux_jev_decision_new` | provider.create(settings: existing Jev decision configuration) → provider.model(method=decision) | Existing core/tests/decision_test.rs and providers/tests/runtime_decision_test.rs retained; ops/decision.rs tests, ops_matrix |
| `aimux_jev_decision_new_with_probability_source` | provider.create(settings: existing Jev probability source) → provider.model(method=decision) | Existing core/tests/decision_test.rs and providers/tests/runtime_decision_test.rs retained; ops/decision.rs tests, ops_matrix |
| `aimux_decision_capabilities` | decision.capabilities | Existing core/tests/decision_test.rs and providers/tests/runtime_decision_test.rs retained; ops/decision.rs tests, ops_matrix |
| `aimux_decide` | decision.decide | Existing core/tests/decision_test.rs and providers/tests/runtime_decision_test.rs retained; ops/decision.rs tests, ops_matrix |
| `aimux_decide_with_abort` | decision.decide + owner-aware Abort handle | Existing core/tests/decision_test.rs and providers/tests/runtime_decision_test.rs retained; ops/decision.rs tests, ops_matrix |
| `aimux_tavily_search_new` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_tavily_search_new_with_base` | provider.create → provider.model (or provider.files for Files) | ops/provider.rs tests; ops_matrix; handle_lifecycle |
| `aimux_search` | search.search | ops/modalities.rs tests; ops_matrix |
| `aimux_codex_refresh` | auth.codexRefresh | ops/misc.rs tests |
| `aimux_init_logging` | runtime.configure (logging / startup proxy settings) | ops/runtime.rs tests |
| `aimux_session_store_init` | sessionStore.create → sessionStore.attach; explicit infer option | ops/session.rs tests |
| `aimux_session_infer_init` | sessionStore.create → sessionStore.attach; explicit infer option | ops/session.rs tests |
| `aimux_session_calls` | sessionStore.calls | ops/session.rs tests |
| `aimux_list_sessions` | sessionStore.list | ops/session.rs tests |
| `aimux_trace_new` | trace.create → middleware.create (trace) → model.wrap; explicit TraceStore handle | ops/trace.rs tests; ops/handles.rs tests; ops/registry.rs tests |
| `aimux_trace_new_audited` | trace.create → middleware.create (trace) → model.wrap; explicit TraceStore handle | ops/trace.rs tests; ops/handles.rs tests; ops/registry.rs tests |
| `aimux_trace_aggregate` | trace.aggregate | ops/trace.rs tests |
| `aimux_trace_session_chain` | trace.sessionChain | ops/trace.rs tests |
| `aimux_trace_session_trajectory` | trace.sessionTrajectory | ops/trace.rs tests |
| `aimux_trace_export_jsonl` | trace.export | ops/trace.rs tests |
| `aimux_trace_clear` | trace.clear | ops/trace.rs tests |
| `aimux_init_recording` | recorder.create → recorder.attach; explicit capture policy | ops/recording.rs tests |
| `aimux_init_recording_ring` | recorder.create → recorder.attach; explicit capture policy | ops/recording.rs tests |
| `aimux_init_recording_ring_default` | recorder.create → recorder.attach; explicit capture policy | ops/recording.rs tests |
| `aimux_recording_stop` | recorder.stop | ops/recording.rs tests |
| `aimux_recording_flush` | recorder.flush; failure-reporting semantics only | ops/recording.rs persistence-error tests; error_envelope_golden |
| `aimux_recording_try_flush` | recorder.flush; failure-reporting semantics only | ops/recording.rs persistence-error tests; error_envelope_golden |
| `aimux_register_providers` | provider.create → registry.create; process-global mutation removed | ops/provider.rs tests; ops/registry.rs tests |
| `aimux_init_proxy` | runtime.configure (logging / startup proxy settings) | ops/runtime.rs tests |
| `aimux_mock_replay_new` | replay.create / replay.capabilities; current recording format only, no legacy JSONL reader | ops/replay.rs fixture tests; providers/tests/replay_test.rs retained |
| `aimux_router_new` | composite.router | ops/composite.rs tests; handle_lifecycle |
| `aimux_moa_new` | composite.moa | ops/composite.rs tests; handle_lifecycle |
