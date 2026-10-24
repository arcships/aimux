use serde_json::{Value, json};

use crate::google::utils::{FinalizeResult, GoogleJsonAccumulator, PartialArg, ProcessResult};

/// One upstream `partialArgs` entry (`{ jsonPath, stringValue, ... }`) as a
/// [`PartialArg`].
fn partial_arg(arg: Value) -> PartialArg {
    PartialArg {
        json_path: arg["jsonPath"].as_str().unwrap_or_default().to_owned(),
        string_value: arg
            .get("stringValue")
            .and_then(Value::as_str)
            .map(str::to_owned),
        number_value: arg.get("numberValue").and_then(Value::as_f64),
        bool_value: arg.get("boolValue").and_then(Value::as_bool),
        null_value: arg.get("nullValue").map(|_| ()),
        will_continue: arg.get("willContinue").and_then(Value::as_bool),
    }
}

/// `accumulator.processPartialArgs([...])`.
fn process(accumulator: &mut GoogleJsonAccumulator, args: Vec<Value>) -> ProcessResult {
    let args: Vec<PartialArg> = args.into_iter().map(partial_arg).collect();
    accumulator
        .process_partial_args(&args)
        .expect("partial args accumulate")
}

/// The upstream `{ currentJSON, textDelta }` object.
fn process_json(result: &ProcessResult) -> Value {
    json!({ "currentJSON": result.current_json, "textDelta": result.text_delta })
}

/// The upstream `{ finalJSON, closingDelta }` object.
fn finalize_json(result: &FinalizeResult) -> Value {
    json!({ "finalJSON": result.final_json, "closingDelta": result.closing_delta })
}

mod google_json_accumulator {
    use super::*;

    mod flat_paths {
        use super::*;

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › flat paths › should accumulate a simple string arg with willContinue
        #[test]
        fn should_accumulate_a_simple_string_arg_with_will_continue() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let result = process(
                &mut accumulator,
                vec![
                    json!({ "jsonPath": "$.location", "stringValue": "Boston", "willContinue": true }),
                ],
            );

            assert_eq!(
                process_json(&result),
                json!({
                    "currentJSON": { "location": "Boston" },
                    "textDelta": r#"{"location":"Boston"#,
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › flat paths › should continue a string arg across multiple chunks
        #[test]
        fn should_continue_a_string_arg_across_multiple_chunks() {
            let mut accumulator = GoogleJsonAccumulator::new();

            process(
                &mut accumulator,
                vec![
                    json!({ "jsonPath": "$.location", "stringValue": "Boston", "willContinue": true }),
                ],
            );

            let continuation_result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.location", "stringValue": ", MA" })],
            );

            assert_eq!(
                process_json(&continuation_result),
                json!({
                    "currentJSON": { "location": "Boston, MA" },
                    "textDelta": ", MA",
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › flat paths › should accumulate a complete string arg (no willContinue)
        #[test]
        fn should_accumulate_a_complete_string_arg_no_will_continue() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.location", "stringValue": "Boston" })],
            );

            assert_eq!(
                process_json(&result),
                json!({
                    "currentJSON": { "location": "Boston" },
                    "textDelta": r#"{"location":"Boston""#,
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › flat paths › should accumulate a number arg
        #[test]
        fn should_accumulate_a_number_arg() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.brightness", "numberValue": 50 })],
            );

            assert_eq!(
                process_json(&result),
                json!({
                    "currentJSON": { "brightness": 50 },
                    "textDelta": r#"{"brightness":50"#,
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › flat paths › should accumulate a boolean arg
        #[test]
        fn should_accumulate_a_boolean_arg() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.enabled", "boolValue": true })],
            );

            assert_eq!(
                process_json(&result),
                json!({
                    "currentJSON": { "enabled": true },
                    "textDelta": r#"{"enabled":true"#,
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › flat paths › should accumulate a null arg
        #[test]
        fn should_accumulate_a_null_arg() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.nickname", "nullValue": {} })],
            );

            assert_eq!(
                process_json(&result),
                json!({
                    "currentJSON": { "nickname": null },
                    "textDelta": r#"{"nickname":null"#,
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › flat paths › should accumulate multiple args with commas between them
        #[test]
        fn should_accumulate_multiple_args_with_commas_between_them() {
            let mut accumulator = GoogleJsonAccumulator::new();

            let first_result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.brightness", "numberValue": 50 })],
            );
            assert_eq!(
                process_json(&first_result),
                json!({
                    "currentJSON": { "brightness": 50 },
                    "textDelta": r#"{"brightness":50"#,
                })
            );

            let second_result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.enabled", "boolValue": true })],
            );
            assert_eq!(
                process_json(&second_result),
                json!({
                    "currentJSON": { "brightness": 50, "enabled": true },
                    "textDelta": r#","enabled":true"#,
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › flat paths › should accumulate multiple args in a single call
        #[test]
        fn should_accumulate_multiple_args_in_a_single_call() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let result = process(
                &mut accumulator,
                vec![
                    json!({ "jsonPath": "$.brightness", "numberValue": 50 }),
                    json!({ "jsonPath": "$.enabled", "boolValue": false }),
                    json!({ "jsonPath": "$.nickname", "nullValue": {} }),
                ],
            );

            assert_eq!(
                process_json(&result),
                json!({
                    "currentJSON": { "brightness": 50, "enabled": false, "nickname": null },
                    "textDelta": r#"{"brightness":50,"enabled":false,"nickname":null"#,
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › flat paths › should escape special characters in continued strings
        #[test]
        fn should_escape_special_characters_in_continued_strings() {
            let mut accumulator = GoogleJsonAccumulator::new();

            process(
                &mut accumulator,
                vec![json!({
                    "jsonPath": "$.query",
                    "stringValue": "Boston \"Lo",
                    "willContinue": true,
                })],
            );

            let continuation_result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.query", "stringValue": "gan\"" })],
            );

            assert_eq!(
                process_json(&continuation_result),
                json!({
                    "currentJSON": { "query": "Boston \"Logan\"" },
                    "textDelta": r#"gan\""#,
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › flat paths › should skip args with empty jsonPath after stripping $. prefix
        #[test]
        fn should_skip_args_with_empty_json_path_after_stripping_prefix() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.", "stringValue": "ignored" })],
            );

            assert_eq!(
                process_json(&result),
                json!({ "currentJSON": {}, "textDelta": "" })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › flat paths › should skip args with no resolvable value
        #[test]
        fn should_skip_args_with_no_resolvable_value() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let result = process(&mut accumulator, vec![json!({ "jsonPath": "$.something" })]);

            assert_eq!(
                process_json(&result),
                json!({ "currentJSON": {}, "textDelta": "" })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › flat paths › should return empty textDelta for empty partialArgs array
        #[test]
        fn should_return_empty_text_delta_for_empty_partial_args_array() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let result = process(&mut accumulator, vec![]);

            assert_eq!(
                process_json(&result),
                json!({ "currentJSON": {}, "textDelta": "" })
            );
        }
    }

    mod prototype_pollution_protection {
        use super::*;

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › prototype pollution protection › should not pollute Object.prototype through __proto__ path segments
        #[test]
        fn should_not_pollute_object_prototype_through_proto_path_segments() {
            // Rust has no `Object.prototype` to pollute; the accumulated
            // object is asserted the same way (an own `__proto__` key).
            let polluted_key = "pollutedByGoogleAccumulatorTest";
            let mut accumulator = GoogleJsonAccumulator::new();
            let result = process(
                &mut accumulator,
                vec![json!({
                    "jsonPath": format!("$.__proto__.{polluted_key}"),
                    "stringValue": "true",
                })],
            );

            assert_eq!(
                result.current_json["__proto__"][polluted_key],
                json!("true")
            );
            assert_eq!(
                result.current_json,
                json!({ "__proto__": { "pollutedByGoogleAccumulatorTest": "true" } })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › prototype pollution protection › should not pollute Object.prototype through constructor.prototype path segments
        #[test]
        fn should_not_pollute_object_prototype_through_constructor_prototype_path_segments() {
            let polluted_key = "pollutedByGoogleConstructorTest";
            let mut accumulator = GoogleJsonAccumulator::new();
            let result = process(
                &mut accumulator,
                vec![json!({
                    "jsonPath": format!("$.constructor.prototype.{polluted_key}"),
                    "stringValue": "true",
                })],
            );

            assert_eq!(
                result.current_json,
                json!({
                    "constructor": {
                        "prototype": { "pollutedByGoogleConstructorTest": "true" },
                    },
                })
            );
        }
    }

    mod nested_paths {
        use super::*;

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › nested paths › should build nested object from dotted jsonPath
        #[test]
        fn should_build_nested_object_from_dotted_json_path() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.recipe.name", "stringValue": "Lasagna" })],
            );

            assert_eq!(
                process_json(&result),
                json!({
                    "currentJSON": { "recipe": { "name": "Lasagna" } },
                    "textDelta": r#"{"recipe":{"name":"Lasagna""#,
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › nested paths › should build nested object with array from indexed jsonPath
        #[test]
        fn should_build_nested_object_with_array_from_indexed_json_path() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let amount_result = process(
                &mut accumulator,
                vec![json!({
                    "jsonPath": "$.recipe.ingredients[0].amount",
                    "stringValue": "16 oz",
                })],
            );
            assert_eq!(
                process_json(&amount_result),
                json!({
                    "currentJSON": { "recipe": { "ingredients": [{ "amount": "16 oz" }] } },
                    "textDelta": r#"{"recipe":{"ingredients":[{"amount":"16 oz""#,
                })
            );

            let name_result = process(
                &mut accumulator,
                vec![json!({
                    "jsonPath": "$.recipe.ingredients[0].name",
                    "stringValue": "Lasagna noodles",
                })],
            );
            assert_eq!(
                process_json(&name_result),
                json!({
                    "currentJSON": {
                        "recipe": {
                            "ingredients": [{ "amount": "16 oz", "name": "Lasagna noodles" }],
                        },
                    },
                    "textDelta": r#","name":"Lasagna noodles""#,
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › nested paths › should accumulate multiple array elements across chunks
        #[test]
        fn should_accumulate_multiple_array_elements_across_chunks() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let mut deltas: Vec<String> = Vec::new();

            let result = process(
                &mut accumulator,
                vec![json!({
                    "jsonPath": "$.recipe.ingredients[0].amount",
                    "stringValue": "16 oz",
                })],
            );
            deltas.push(result.text_delta);

            let result = process(
                &mut accumulator,
                vec![json!({
                    "jsonPath": "$.recipe.ingredients[0].name",
                    "stringValue": "Noodles",
                })],
            );
            deltas.push(result.text_delta);

            let result = process(
                &mut accumulator,
                vec![json!({
                    "jsonPath": "$.recipe.ingredients[1].amount",
                    "stringValue": "1 lb",
                })],
            );
            deltas.push(result.text_delta.clone());
            assert_eq!(result.text_delta, r#"},{"amount":"1 lb""#);

            let result = process(
                &mut accumulator,
                vec![json!({
                    "jsonPath": "$.recipe.ingredients[1].name",
                    "stringValue": "Beef",
                })],
            );
            deltas.push(result.text_delta.clone());
            assert_eq!(result.text_delta, r#","name":"Beef""#);

            assert_eq!(
                result.current_json,
                json!({
                    "recipe": {
                        "ingredients": [
                            { "amount": "16 oz", "name": "Noodles" },
                            { "amount": "1 lb", "name": "Beef" },
                        ],
                    },
                })
            );

            let FinalizeResult {
                final_json,
                closing_delta,
            } = accumulator.finalize();
            deltas.push(closing_delta);
            assert_eq!(deltas.join(""), final_json);
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › nested paths › should handle string continuation on nested paths
        #[test]
        fn should_handle_string_continuation_on_nested_paths() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let start_result = process(
                &mut accumulator,
                vec![json!({
                    "jsonPath": "$.recipe.steps[0]",
                    "stringValue": "Preheat oven",
                    "willContinue": true,
                })],
            );
            assert_eq!(
                start_result.text_delta,
                r#"{"recipe":{"steps":["Preheat oven"#
            );

            let continuation_result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.recipe.steps[0]", "stringValue": " to 375°F." })],
            );
            assert_eq!(
                process_json(&continuation_result),
                json!({
                    "currentJSON": { "recipe": { "steps": ["Preheat oven to 375°F."] } },
                    "textDelta": " to 375°F.",
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › nested paths › should handle mixed nested and flat paths
        #[test]
        fn should_handle_mixed_nested_and_flat_paths() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let location_result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.location", "stringValue": "Boston" })],
            );
            assert_eq!(location_result.text_delta, r#"{"location":"Boston""#);

            let details_result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.details.zip", "stringValue": "02101" })],
            );
            assert_eq!(
                process_json(&details_result),
                json!({
                    "currentJSON": { "details": { "zip": "02101" }, "location": "Boston" },
                    "textDelta": r#","details":{"zip":"02101""#,
                })
            );

            assert_eq!(
                finalize_json(&accumulator.finalize()),
                json!({
                    "closingDelta": "}}",
                    "finalJSON": r#"{"location":"Boston","details":{"zip":"02101"}}"#,
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › nested paths › should handle array elements that are direct string values
        #[test]
        fn should_handle_array_elements_that_are_direct_string_values() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let first_step = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.steps[0]", "stringValue": "Step one" })],
            );
            assert_eq!(first_step.text_delta, r#"{"steps":["Step one""#);

            let second_step = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.steps[1]", "stringValue": "Step two" })],
            );
            assert_eq!(
                process_json(&second_step),
                json!({
                    "currentJSON": { "steps": ["Step one", "Step two"] },
                    "textDelta": r#","Step two""#,
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › nested paths › should handle deeply nested paths
        #[test]
        fn should_handle_deeply_nested_paths() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.a.b.c.d", "stringValue": "deep" })],
            );

            assert_eq!(
                process_json(&result),
                json!({
                    "currentJSON": { "a": { "b": { "c": { "d": "deep" } } } },
                    "textDelta": r#"{"a":{"b":{"c":{"d":"deep""#,
                })
            );

            assert_eq!(
                finalize_json(&accumulator.finalize()),
                json!({
                    "closingDelta": "}}}}",
                    "finalJSON": r#"{"a":{"b":{"c":{"d":"deep"}}}}"#,
                })
            );
        }
    }

    mod finalize {
        use super::*;

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › finalize › should produce closing delta for a continued string
        #[test]
        fn should_produce_closing_delta_for_a_continued_string() {
            let mut accumulator = GoogleJsonAccumulator::new();
            process(
                &mut accumulator,
                vec![
                    json!({ "jsonPath": "$.location", "stringValue": "Boston", "willContinue": true }),
                ],
            );

            assert_eq!(
                finalize_json(&accumulator.finalize()),
                json!({
                    "closingDelta": r#""}"#,
                    "finalJSON": r#"{"location":"Boston"}"#,
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › finalize › should produce closing delta for a complete string
        #[test]
        fn should_produce_closing_delta_for_a_complete_string() {
            let mut accumulator = GoogleJsonAccumulator::new();
            process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.location", "stringValue": "Boston" })],
            );

            assert_eq!(
                finalize_json(&accumulator.finalize()),
                json!({
                    "closingDelta": "}",
                    "finalJSON": r#"{"location":"Boston"}"#,
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › finalize › should produce closing delta for multiple args
        #[test]
        fn should_produce_closing_delta_for_multiple_args() {
            let mut accumulator = GoogleJsonAccumulator::new();
            process(
                &mut accumulator,
                vec![
                    json!({ "jsonPath": "$.brightness", "numberValue": 50 }),
                    json!({ "jsonPath": "$.enabled", "boolValue": true }),
                ],
            );

            assert_eq!(
                finalize_json(&accumulator.finalize()),
                json!({
                    "closingDelta": "}",
                    "finalJSON": r#"{"brightness":50,"enabled":true}"#,
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › finalize › should produce closing delta for continued string with continuation
        #[test]
        fn should_produce_closing_delta_for_continued_string_with_continuation() {
            let mut accumulator = GoogleJsonAccumulator::new();
            process(
                &mut accumulator,
                vec![
                    json!({ "jsonPath": "$.location", "stringValue": "Boston", "willContinue": true }),
                ],
            );
            process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.location", "stringValue": ", MA" })],
            );

            assert_eq!(
                finalize_json(&accumulator.finalize()),
                json!({
                    "closingDelta": r#""}"#,
                    "finalJSON": r#"{"location":"Boston, MA"}"#,
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › finalize › should handle empty accumulator
        #[test]
        fn should_handle_empty_accumulator() {
            let accumulator = GoogleJsonAccumulator::new();
            assert_eq!(
                finalize_json(&accumulator.finalize()),
                json!({ "closingDelta": "{}", "finalJSON": "{}" })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › finalize › should finalize nested structure to proper JSON
        #[test]
        fn should_finalize_nested_structure_to_proper_json() {
            let mut accumulator = GoogleJsonAccumulator::new();
            process(
                &mut accumulator,
                vec![json!({
                    "jsonPath": "$.recipe.ingredients[0].name",
                    "stringValue": "Noodles",
                })],
            );
            process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.recipe.name", "stringValue": "Lasagna" })],
            );

            let FinalizeResult { final_json, .. } = accumulator.finalize();
            assert_eq!(
                serde_json::from_str::<Value>(&final_json).unwrap(),
                json!({
                    "recipe": {
                        "ingredients": [{ "name": "Noodles" }],
                        "name": "Lasagna",
                    },
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › finalize › should finalize nested arrays with string continuation
        #[test]
        fn should_finalize_nested_arrays_with_string_continuation() {
            let mut accumulator = GoogleJsonAccumulator::new();
            process(
                &mut accumulator,
                vec![json!({
                    "jsonPath": "$.recipe.steps[0]",
                    "stringValue": "Preheat",
                    "willContinue": true,
                })],
            );
            process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.recipe.steps[0]", "stringValue": " oven." })],
            );
            process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.recipe.steps[1]", "stringValue": "Cook." })],
            );

            let FinalizeResult { final_json, .. } = accumulator.finalize();
            assert_eq!(
                serde_json::from_str::<Value>(&final_json).unwrap(),
                json!({ "recipe": { "steps": ["Preheat oven.", "Cook."] } })
            );
        }
    }

    mod concatenation_invariant {
        use super::*;

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › concatenation invariant › flat args: concatenated deltas + closingDelta === JSON.stringify
        #[test]
        fn flat_args_concatenated_deltas_closing_delta_json_stringify() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let mut deltas: Vec<String> = Vec::new();

            let result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.brightness", "numberValue": 50 })],
            );
            deltas.push(result.text_delta);

            let result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.enabled", "boolValue": true })],
            );
            deltas.push(result.text_delta);

            let result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.name", "stringValue": "test" })],
            );
            deltas.push(result.text_delta);

            let FinalizeResult {
                final_json,
                closing_delta,
            } = accumulator.finalize();
            deltas.push(closing_delta);
            assert_eq!(deltas.join(""), final_json);
            assert_eq!(
                serde_json::from_str::<Value>(&final_json).unwrap(),
                json!({ "brightness": 50, "enabled": true, "name": "test" })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › concatenation invariant › nested args: concatenated deltas + closingDelta === JSON.stringify
        #[test]
        fn nested_args_concatenated_deltas_closing_delta_json_stringify() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let mut deltas: Vec<String> = Vec::new();

            for arg in [
                json!({ "jsonPath": "$.recipe.ingredients[0].amount", "stringValue": "16 oz" }),
                json!({ "jsonPath": "$.recipe.ingredients[0].name", "stringValue": "Noodles" }),
                json!({ "jsonPath": "$.recipe.ingredients[1].amount", "stringValue": "1 lb" }),
                json!({ "jsonPath": "$.recipe.ingredients[1].name", "stringValue": "Beef" }),
                json!({ "jsonPath": "$.recipe.name", "stringValue": "Lasagna" }),
                json!({
                    "jsonPath": "$.recipe.steps[0]",
                    "stringValue": "Preheat",
                    "willContinue": true,
                }),
                json!({ "jsonPath": "$.recipe.steps[0]", "stringValue": " oven." }),
                json!({ "jsonPath": "$.recipe.steps[1]", "stringValue": "Cook." }),
            ] {
                let result = process(&mut accumulator, vec![arg]);
                deltas.push(result.text_delta);
            }

            let FinalizeResult {
                final_json,
                closing_delta,
            } = accumulator.finalize();
            deltas.push(closing_delta);
            assert_eq!(deltas.join(""), final_json);
            assert_eq!(
                serde_json::from_str::<Value>(&final_json).unwrap(),
                json!({
                    "recipe": {
                        "ingredients": [
                            { "amount": "16 oz", "name": "Noodles" },
                            { "amount": "1 lb", "name": "Beef" },
                        ],
                        "name": "Lasagna",
                        "steps": ["Preheat oven.", "Cook."],
                    },
                })
            );
        }

        /// upstream: @ai-sdk/google src/google-json-accumulator.test.ts › GoogleJSONAccumulator › concatenation invariant › willContinue strings: concatenated deltas + closingDelta === JSON.stringify
        #[test]
        fn will_continue_strings_concatenated_deltas_closing_delta_json_stringify() {
            let mut accumulator = GoogleJsonAccumulator::new();
            let mut deltas: Vec<String> = Vec::new();

            let result = process(
                &mut accumulator,
                vec![
                    json!({ "jsonPath": "$.location", "stringValue": "Bos", "willContinue": true }),
                ],
            );
            deltas.push(result.text_delta);

            let result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.location", "stringValue": "ton" })],
            );
            deltas.push(result.text_delta);

            let result = process(
                &mut accumulator,
                vec![json!({ "jsonPath": "$.count", "numberValue": 42 })],
            );
            deltas.push(result.text_delta);

            let FinalizeResult {
                final_json,
                closing_delta,
            } = accumulator.finalize();
            deltas.push(closing_delta);
            assert_eq!(deltas.join(""), final_json);
        }
    }
}
