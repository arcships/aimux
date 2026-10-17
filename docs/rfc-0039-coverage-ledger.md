# RFC-0039 coverage ledger

This Markdown document is the coverage ledger. It records the 130 retired or
retained C exports and operation-level evidence, plus representative binding
facades and tests. It does not enumerate every former language declaration,
overload, DTO, builder or codec, and does not claim individual replacement
coverage for them. The C inventory baseline is
`7dbce97af1a50017c4598bf2a684bfec4f50029d`.

## Reading the ledger

The C rows map old exports to current operations or ABI primitives; the
capability and binding tables identify existing tests (`path#name`) and current
facade sources. ` + ` is a migration recipe, not a new op. Old names are
historical inventory records, not assertions that those declarations still
exist in this checkout or compatibility aliases.

- **mapped:** a current operation/ABI path and related existing test are identified;
  this is not complete behavioral equivalence or a test-pass claim.
- **partial:** representative coverage exists, but old per-method/DTO equivalence
  or adapter evidence is incomplete; the note states the limitation.
- **gap:** a specific former capability is not supplied by the replacement path.

The bindings use the actual shared dispatcher/ABI, with Python Client/Handle,
Node Client/resources, Go Create/Handle, Java Aimux/NativeHandle (also used by
Kotlin), Swift AimuxRuntime/AimuxHandle and Dart Aimux/OpsHandle. Generic dispatch
is intentional: a ledger recipe does not invent a convenience method.
Representative DTO tests are explicitly partial, not asserted coverage of every
builder or every raw/typed round trip. Business DTO generation remains G-d.

## Binding evidence

These sources expose current dispatch facades. Each test establishes only the
behavior named, not equivalence for every former API in that language; complete
replacement coverage remains a gap under G-d/G-i, with offline mock generation
tracked separately as G-k. Test presence is not an execution result.

| Interface | Current facade source | Representative existing test |
|---|---|---|
| Go | `bindings/go/aimux.go` (`Create`, `Handle.Call`); `bindings/go/operation.go` (`Handle.StartOperation`) | `bindings/go/native_test.go#TestNativeDispatchAndCompositeLifetime` |
| Java | `bindings/java/src/main/java/ai/arcships/aimux/Aimux.java` (`Aimux`); `bindings/java/src/main/java/ai/arcships/aimux/NativeHandle.java` (`NativeHandle`) | `bindings/java/src/test/java/ai/arcships/aimux/AbiContractTest.java#allEightModalitiesAndFilesUseTheSameOwnedProviderPrimitives` |
| Kotlin | `bindings/kotlin/src/main/kotlin/ai/arcships/aimux/kotlin/Coroutines.kt` (`aimuxCall`, `aimuxFlow`), using Java resources | `bindings/kotlin/src/test/kotlin/ai/arcships/aimux/kotlin/NativeIntegrationTest.kt#real manifest loads and Java provider model objects retain identity` |
| Swift | `bindings/swift/Sources/Aimux/Aimux.swift` (`AimuxRuntime`, `AimuxHandle`) | `bindings/swift/Tests/AimuxTests/NativeSmokeTests.swift#providerAndModel` |
| Dart | `bindings/flutter/lib/aimux.dart` (`Aimux`, `OpsHandle`) | `bindings/flutter/test/native_runtime_test.dart#provider/model handles retain children after provider close` |
| Node | `bindings/node/src/index.ts` (`Client`, `Resource`); `bindings/node/src/native.ts` (`Runtime`) | `bindings/node/__test__/contract.native.ts#typed lifecycle uses the real provider, registry, files, composite and devtools DTOs` |
| Python | `bindings/python/python/aimux/__init__.py` (`Client`, `Handle`, `EventStream`) | `bindings/python/tests/test_ops_integration.py#test_control_dtos_reject_unknown_fields` |

## Capability evidence

Every operation used in a recipe has an existing related test below. Registration
tests establish only registration; other family tests establish behavior as
described by their names. A successful provider/network request, every business
DTO and all language projections require additional evidence (G-i).

| New operation | Existing test |
|---|---|
| `abort.create` | `aimux-ops/tests/handles.rs#abort_is_idempotent_and_signals_another_thread` |
| `abort.signal` | `aimux-ops/tests/handles.rs#abort_is_idempotent_and_signals_another_thread` |
| `auth.codexRefresh` | `aimux-ops/tests/misc.rs#invalid_auth_request_is_projected_once_without_token_values` |
| `catalogue.fetch` | `aimux-ops/src/ops/misc.rs#catalogue_rejects_untrusted_sources_before_io` |
| `composite.moa` | `aimux-ops/tests/composite.rs#moa_retains_references_and_aggregator_and_completes` |
| `composite.router` | `aimux-ops/tests/composite.rs#dropped_router_children_keep_working_with_independent_retry_budgets` |
| `credential.create` | `aimux-ops/tests/handles.rs#credential_creation_is_future_and_never_exports_supplied_material` |
| `decision.capabilities` | `aimux-ops/tests/decision.rs#capabilities_match_existing_providers_and_children_survive_parent_drop` |
| `decision.decide` | `aimux-ops/tests/decision.rs#decide_matches_core_for_all_state_shapes_and_question_types` |
| `embedding.embed` | `aimux-ops/src/ops/modalities.rs#embedding_single` |
| `embedding.embedMany` | `aimux-ops/src/ops/modalities.rs#embedding_many_preserves_all_inputs` |
| `files.upload` | `aimux-ops/tests/files.rs#uploads_canonical_base64_and_preserves_options_and_context_abort` |
| `handle.drop` | `aimux-ops/tests/ops_matrix.rs#every_manifest_op_crosses_every_kind_and_invalid_handle_state` |
| `image.generate` | `aimux-ops/src/ops/modalities.rs#image_generation` |
| `middleware.create` | `aimux-ops/tests/handles.rs#trace_middleware_creation_retains_store_and_audit_configuration` |
| `model.info` | `aimux-ops/src/ops/registry.rs#direct_models_have_no_invented_source_and_empty_wrap_retains_modality` |
| `model.resolve` | `aimux-ops/src/ops/registry.rs#resolution_preserves_exact_ids_and_reports_safe_source` |
| `model.wrap` | `aimux-ops/tests/registry.rs#language_and_image_middleware_use_core_order_and_keep_source` |
| `operation.cancel` | `aimux-ops/tests/operation.rs#native_binary_and_json_poll_share_the_registered_operation_service` |
| `operation.inputDone` | `aimux-ops/tests/operation.rs#native_binary_and_json_poll_share_the_registered_operation_service` |
| `operation.next` | `aimux-ops/tests/operation.rs#native_binary_and_json_poll_share_the_registered_operation_service` |
| `operation.push` | `aimux-ops/tests/operation.rs#native_binary_and_json_poll_share_the_registered_operation_service` |
| `operation.status` | `aimux-ops/tests/operation.rs#native_binary_and_json_poll_share_the_registered_operation_service` |
| `output.toOpenAI` | `aimux-ops/tests/misc.rs#output_uses_explicit_metadata_and_model_fallback_without_generation` |
| `provider.create` | `aimux-ops/src/ops/provider.rs#create_uses_the_existing_package_factory` |
| `provider.custom` | `aimux-ops/src/ops/provider.rs#custom_keeps_aliases_default_method_and_strong_references` |
| `provider.default` | `aimux-ops/src/ops/provider.rs#default_uses_preset_descriptor_and_requires_global_scope` |
| `provider.files` | `aimux-ops/tests/files.rs#provider_files_returns_an_owned_handle_that_survives_parent_drop` |
| `provider.invoke` | `aimux-ops/src/ops/provider.rs#invoke_returns_only_runtime_discovery_and_forbids_arbitrary_rpc` |
| `provider.model` | `aimux-ops/src/ops/provider.rs#model_returns_all_eight_exact_handle_kinds` |
| `provider.wrap` | `aimux-ops/src/ops/provider.rs#wrap_applies_each_middleware_and_keeps_files_and_discovery` |
| `recorder.attach` | `aimux-ops/tests/recording.rs#ring_lifecycle_keeps_snapshots_and_replacement_and_dropped_handles` |
| `recorder.create` | `aimux-ops/tests/recording.rs#ring_lifecycle_keeps_snapshots_and_replacement_and_dropped_handles` |
| `recorder.export` | `aimux-ops/tests/recording.rs#ring_lifecycle_keeps_snapshots_and_replacement_and_dropped_handles` |
| `recorder.flush` | `aimux-ops/tests/recording.rs#ring_lifecycle_keeps_snapshots_and_replacement_and_dropped_handles` |
| `recorder.stats` | `aimux-ops/tests/recording.rs#ring_lifecycle_keeps_snapshots_and_replacement_and_dropped_handles` |
| `recorder.stop` | `aimux-ops/tests/recording.rs#ring_lifecycle_keeps_snapshots_and_replacement_and_dropped_handles` |
| `registry.create` | `aimux-ops/src/ops/registry.rs#duplicate_keys_are_rejected` |
| `registry.provider` | `aimux-ops/src/ops/registry.rs#registry_retains_exact_provider_after_original_handle_drop` |
| `replay.capabilities` | `aimux-ops/tests/replay.rs#future_capabilities_return_projected_unsupported_even_with_complete_http_data` |
| `replay.create` | `aimux-ops/tests/replay.rs#creation_inspection_and_planning_use_real_session_payloads` |
| `replay.dryRun` | `aimux-ops/tests/replay.rs#creation_inspection_and_planning_use_real_session_payloads` |
| `replay.inspect` | `aimux-ops/tests/replay.rs#creation_inspection_and_planning_use_real_session_payloads` |
| `replay.live` | `aimux-ops/tests/replay.rs#language_live_matches_the_existing_core_result_twice` |
| `reranking.rerank` | `aimux-ops/src/ops/modalities.rs#reranking_text_documents` |
| `runtime.configure` | `aimux-ops/tests/runtime.rs#startup_configuration_uses_native_freeze_authorities` |
| `runtime.describe` | `aimux-ops/tests/runtime.rs#description_matches_the_real_manifest_and_digest` |
| `search.search` | `aimux-ops/src/ops/modalities.rs#search_extension` |
| `sessionStore.attach` | `aimux-ops/tests/session.rs#create_and_attach_preserve_process_global_infer_and_capture_behavior` |
| `sessionStore.calls` | `aimux-ops/tests/session.rs#queries_use_selected_store_and_preserve_core_wire_and_bounds` |
| `sessionStore.create` | `aimux-ops/tests/session.rs#create_and_attach_preserve_process_global_infer_and_capture_behavior` |
| `sessionStore.list` | `aimux-ops/tests/session.rs#queries_use_selected_store_and_preserve_core_wire_and_bounds` |
| `speech.generate` | `aimux-ops/src/ops/modalities.rs#speech_generation` |
| `text.consume` | `aimux-ops/src/ops/text.rs#real_dispatch_runs_all_six_text_operations` |
| `text.generate` | `aimux-ops/src/ops/text.rs#real_dispatch_runs_all_six_text_operations` |
| `text.generateObject` | `aimux-ops/src/ops/text.rs#real_dispatch_runs_all_six_text_operations` |
| `text.generateOpenAI` | `aimux-ops/src/ops/text.rs#real_dispatch_runs_all_six_text_operations` |
| `text.stream` | `aimux-ops/src/ops/text.rs#real_dispatch_runs_all_six_text_operations` |
| `text.streamOpenAI` | `aimux-ops/src/ops/text.rs#real_dispatch_runs_all_six_text_operations` |
| `toolRepair.apply` | `aimux-ops/src/ops/misc.rs#apply_matches_core_for_all_reply_variants` |
| `toolRepair.applyResult` | `aimux-ops/src/ops/misc.rs#apply_result_updates_calls_and_response_messages` |
| `toolRepair.context` | `aimux-ops/src/ops/misc.rs#context_matches_core_and_preserves_no_tools_null` |
| `trace.aggregate` | `aimux-ops/tests/trace.rs#aggregate_matches_six_canonical_filter_queries` |
| `trace.clear` | `aimux-ops/tests/trace.rs#sessions_export_and_idempotent_clear_preserve_ffi_query_semantics` |
| `trace.create` | `aimux-ops/tests/trace.rs#create_uses_real_store_capacity_defaults_and_owned_handle` |
| `trace.export` | `aimux-ops/tests/trace.rs#sessions_export_and_idempotent_clear_preserve_ffi_query_semantics` |
| `trace.sessionChain` | `aimux-ops/tests/trace.rs#sessions_export_and_idempotent_clear_preserve_ffi_query_semantics` |
| `trace.sessionTrajectory` | `aimux-ops/tests/trace.rs#sessions_export_and_idempotent_clear_preserve_ffi_query_semantics` |
| `transcription.generate` | `aimux-ops/src/ops/modalities.rs#transcription_base64_bytes` |
| `transcription.start` | `aimux-ops/tests/operation.rs#native_binary_and_json_poll_share_the_registered_operation_service` |
| `transport.create` | `aimux-ops/tests/handles.rs#transports_have_precise_kinds_and_decimal_ids` |
| `video.generate` | `aimux-ops/src/ops/modalities.rs#video_uses_core_batching_and_polling` |

## C export inventory

Four old names remain primitives: aimux_error_free, aimux_error_code,
aimux_error_message and aimux_free_string. The other 126 old exports are retired;
the current 14-symbol ABI adds ten names. The actual-cdylib allowlist is tested
by `aimux-ffi/tests/exports_smoke_test.rs#cdylib_exports_only_abi_primitives`.
The existing [ABI migration inventory](../aimux-ffi/ABI-MIGRATION.md) lists the
same 130 old names. The capability table supplies related operation evidence;
the export allowlist alone does not establish old per-export equivalence.

| Old export | Replacement | Status |
|---|---|---|
| `aimux_error_free` | `abi: aimux_error_free` | mapped |
| `aimux_error_code` | `abi: aimux_error_code` | mapped |
| `aimux_error_message` | `abi: aimux_error_message` | mapped |
| `aimux_error_status` | `abi: aimux_error_json` | mapped |
| `aimux_error_retry_ms` | `abi: aimux_error_json` | mapped |
| `aimux_error_retryable` | `abi: aimux_error_json` | mapped |
| `aimux_error_provider_code` | `abi: aimux_error_json` | mapped |
| `aimux_error_provider_message` | `abi: aimux_error_json` | mapped |
| `aimux_error_response_body` | `abi: aimux_error_json` | mapped |
| `aimux_error_url` | `abi: aimux_error_json` | mapped |
| `aimux_error_request_body_values` | `abi: aimux_error_json` | mapped |
| `aimux_error_response_headers` | `abi: aimux_error_json` | mapped |
| `aimux_error_provider_data` | `abi: aimux_error_json` | mapped |
| `aimux_error_retry_reason` | `abi: aimux_error_json` | mapped |
| `aimux_error_retry_count` | `abi: aimux_error_json` | mapped |
| `aimux_error_retry_error_at` | `abi: aimux_error_json` | mapped |
| `aimux_error_model_id` | `abi: aimux_error_json` | mapped |
| `aimux_error_model_type` | `abi: aimux_error_json` | mapped |
| `aimux_error_provider_id` | `abi: aimux_error_json` | mapped |
| `aimux_error_tool_name` | `abi: aimux_error_json` | mapped |
| `aimux_error_available_tools` | `abi: aimux_error_json` | mapped |
| `aimux_error_tool_input` | `abi: aimux_error_json` | mapped |
| `aimux_error_original_error` | `abi: aimux_error_json` | mapped |
| `aimux_openai_new` | `provider.create + provider.model` | mapped |
| `aimux_openai_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_anthropic_new` | `provider.create + provider.model` | mapped |
| `aimux_anthropic_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_anthropic_aws_new` | `provider.create + provider.model` | mapped |
| `aimux_anthropic_aws_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_azure_new` | `provider.create + provider.model` | mapped |
| `aimux_azure_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_bedrock_new` | `provider.create + provider.model` | mapped |
| `aimux_bedrock_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_vertex_new` | `provider.create + provider.model` | mapped |
| `aimux_vertex_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_cohere_new` | `provider.create + provider.model` | mapped |
| `aimux_cohere_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_mistral_new` | `provider.create + provider.model` | mapped |
| `aimux_mistral_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_xai_new` | `provider.create + provider.model` | mapped |
| `aimux_xai_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_provider_new` | `provider.create + provider.model` | mapped |
| `aimux_provider_from_env` | `provider.default + provider.model` | mapped |
| `aimux_provider_handle_new` | `provider.create` | mapped |
| `aimux_provider_list_models` | `provider.invoke` | mapped |
| `aimux_provider_model` | `provider.model` | mapped |
| `aimux_get_model_specs` | `catalogue.fetch` | mapped |
| `aimux_generate_text` | `text.generate` | mapped |
| `aimux_generate_object` | `text.generateObject` | mapped |
| `aimux_consume_stream_text` | `text.consume` | mapped |
| `aimux_stream_text` | `text.stream` | mapped |
| `aimux_abort_signal_new` | `abort.create` | mapped |
| `aimux_abort_signal_abort` | `abort.signal` | mapped |
| `aimux_abort_signal_drop` | `handle.drop` | mapped |
| `aimux_stream_text_with_abort` | `text.stream` | mapped |
| `aimux_tool_call_repair_context` | `toolRepair.context` | mapped |
| `aimux_apply_tool_call_repair` | `toolRepair.apply` | mapped |
| `aimux_apply_tool_call_repair_to_result` | `toolRepair.applyResult` | mapped |
| `aimux_generate_text_result_as_openai` | `output.toOpenAI` | mapped |
| `aimux_generate_text_as_openai` | `text.generateOpenAI` | mapped |
| `aimux_stream_text_as_openai` | `text.streamOpenAI` | mapped |
| `aimux_stream_text_as_openai_with_abort` | `text.streamOpenAI` | mapped |
| `aimux_drop_handle` | `handle.drop` | mapped |
| `aimux_drop_handle_ptr` | `abi: aimux_handle_finalizer` | mapped |
| `aimux_free_string` | `abi: aimux_free_string` | mapped |
| `aimux_openai_embedding_new` | `provider.create + provider.model` | mapped |
| `aimux_openai_embedding_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_cohere_embedding_new` | `provider.create + provider.model` | mapped |
| `aimux_cohere_embedding_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_google_embedding_new` | `provider.create + provider.model` | mapped |
| `aimux_google_embedding_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_embed` | `embedding.embedMany` | mapped |
| `aimux_openai_speech_new` | `provider.create + provider.model` | mapped |
| `aimux_openai_speech_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_speech_generate` | `speech.generate` | mapped |
| `aimux_openai_image_new` | `provider.create + provider.model` | mapped |
| `aimux_openai_image_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_google_image_new` | `provider.create + provider.model` | mapped |
| `aimux_google_image_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_image_generate` | `image.generate` | mapped |
| `aimux_openai_transcription_new` | `provider.create + provider.model` | mapped |
| `aimux_openai_transcription_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_transcription_generate` | `transcription.generate` | mapped |
| `aimux_transcription_session_new` | `transcription.start` | mapped |
| `aimux_transcription_push_audio` | `operation.push` | mapped |
| `aimux_transcription_input_done` | `operation.inputDone` | mapped |
| `aimux_transcription_next_part` | `operation.next` | mapped |
| `aimux_transcription_session_drop` | `handle.drop` | mapped |
| `aimux_openai_files_new` | `provider.create + provider.files` | mapped |
| `aimux_openai_files_new_with_base` | `provider.create + provider.files` | mapped |
| `aimux_file_upload` | `files.upload` | mapped |
| `aimux_cohere_reranking_new` | `provider.create + provider.model` | mapped |
| `aimux_cohere_reranking_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_rerank` | `reranking.rerank` | mapped |
| `aimux_google_video_new` | `provider.create + provider.model` | mapped |
| `aimux_google_video_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_video_generate` | `video.generate` | mapped |
| `aimux_provider_decision_model` | `provider.model` | mapped |
| `aimux_jev_decision_new` | `provider.create + provider.model` | mapped |
| `aimux_jev_decision_new_with_probability_source` | `provider.create + provider.model` | mapped |
| `aimux_decision_capabilities` | `decision.capabilities` | mapped |
| `aimux_decide` | `decision.decide` | mapped |
| `aimux_decide_with_abort` | `decision.decide` | mapped |
| `aimux_tavily_search_new` | `provider.create + provider.model` | mapped |
| `aimux_tavily_search_new_with_base` | `provider.create + provider.model` | mapped |
| `aimux_search` | `search.search` | mapped |
| `aimux_codex_refresh` | `auth.codexRefresh` | partial |
| `aimux_init_logging` | `runtime.configure` | mapped |
| `aimux_session_store_init` | `sessionStore.create + sessionStore.attach` | mapped |
| `aimux_session_infer_init` | `sessionStore.create + sessionStore.attach` | mapped |
| `aimux_session_calls` | `sessionStore.calls` | mapped |
| `aimux_list_sessions` | `sessionStore.list` | mapped |
| `aimux_trace_new` | `trace.create + middleware.create + model.wrap` | mapped |
| `aimux_trace_new_audited` | `trace.create + middleware.create + model.wrap` | mapped |
| `aimux_trace_aggregate` | `trace.aggregate` | mapped |
| `aimux_trace_session_chain` | `trace.sessionChain` | mapped |
| `aimux_trace_session_trajectory` | `trace.sessionTrajectory` | mapped |
| `aimux_trace_export_jsonl` | `trace.export` | mapped |
| `aimux_trace_clear` | `trace.clear` | mapped |
| `aimux_init_recording` | `recorder.create + recorder.attach` | mapped |
| `aimux_init_recording_ring` | `recorder.create + recorder.attach` | mapped |
| `aimux_init_recording_ring_default` | `recorder.create + recorder.attach` | mapped |
| `aimux_recording_stop` | `recorder.stop` | mapped |
| `aimux_recording_flush` | `recorder.flush` | mapped |
| `aimux_recording_try_flush` | `recorder.flush` | mapped |
| `aimux_register_providers` | `provider.create + registry.create` | mapped |
| `aimux_init_proxy` | `runtime.configure` | mapped |
| `aimux_mock_replay_new` | `replay.create + replay.inspect + replay.dryRun + replay.live` | gap |
| `aimux_router_new` | `composite.router` | mapped |
| `aimux_moa_new` | `composite.moa` | mapped |

## Explicit limits of replacement coverage

Decision is no longer a missing op-table entry: model selection with method
decision, capabilities and decide are implemented and tested against core.
The old Jev constructor options are reachable through provider.create.
Trace wrapping now uses canonical TraceLayer and shared tests cover real
generation/streaming; the Node native lifecycle test expects trace middleware
creation and model wrapping to succeed.
The same existing provider factory supplies package construction; no separate
provider table is introduced by this ledger.

The old mock-replay constructor is **not fully replaced** by replay.create.
It now returns ReplaySession, which cannot be passed to text.generate/stream;
replay.live needs a separate live Model/Registry target. Offline mock Language
generation remains G-k. Future replay transport capabilities cannot be used as
a workaround because replay.capabilities currently fails (G-c).

Writers use schema 2 and readers accept schemas 1 and 2; migration to schema 3
and removal of old-schema reading are planned as the final step of this series
under RFC-0041 (G-b). Process-wide capture remains G-a. Generated-output tests
exist, but this audit does not claim a successful run; per-business-type hash
checks remain G-h. Installed assets, platform
execution, successful token refresh, exhaustive eight-language wire vectors and
old-only-test equivalence are not proved by this inventory (G-d/G-e/G-i).
See the [registered gaps](rfc-0039-implementation.md#registered-gaps) and
[conformance matrix](rfc-0039-conformance.md).
