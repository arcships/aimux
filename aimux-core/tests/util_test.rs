//! Behaviour-faithful Rust translations of the pure-function tests under
//! `reference/ai/packages/ai/src/util/`:
//!
//! - `fix-json.test.ts`              (47 `test` blocks)
//! - `parse-partial-json.test.ts`    (4 `it` blocks)
//! - `cosine-similarity.test.ts`     (5 `it` blocks)
//!
//! Each TS `it`/`test` maps to one Rust `#[test]`. Assertions are translated
//! 1:1; the only omissions are mock-interaction assertions (`expect(...).
//! toHaveBeenCalledWith(...)`) from `parse-partial-json.test.ts`, which have no
//! Rust equivalent because the port uses real `serde_json` parsing instead of
//! the mocked `safeParseJSON`. Such omissions are called out in comments.

use aimux_core::util::{
    ParsePartialJsonState, UtilError, cosine_similarity, fix_json, parse_partial_json,
};
use serde_json::json;

// ===========================================================================
// fix-json.test.ts
// ===========================================================================

#[test]
fn fix_json_should_handle_empty_input() {
    assert_eq!(fix_json(""), "");
}

mod fix_json_literals {
    use super::*;

    #[test]
    fn should_handle_incomplete_null() {
        assert_eq!(fix_json("nul"), "null");
    }

    #[test]
    fn should_handle_incomplete_true() {
        assert_eq!(fix_json("t"), "true");
    }

    #[test]
    fn should_handle_incomplete_false() {
        assert_eq!(fix_json("fals"), "false");
    }
}

mod fix_json_number {
    use super::*;

    #[test]
    fn should_handle_incomplete_numbers() {
        assert_eq!(fix_json("12."), "12");
    }

    #[test]
    fn should_handle_numbers_with_dot() {
        assert_eq!(fix_json("12.2"), "12.2");
    }

    #[test]
    fn should_handle_negative_numbers() {
        assert_eq!(fix_json("-12"), "-12");
    }

    #[test]
    fn should_handle_incomplete_negative_numbers() {
        assert_eq!(fix_json("-"), "");
    }

    #[test]
    fn should_handle_e_notation_numbers() {
        assert_eq!(fix_json("2.5e"), "2.5");
        assert_eq!(fix_json("2.5e-"), "2.5");
        assert_eq!(fix_json("2.5e3"), "2.5e3");
        assert_eq!(fix_json("-2.5e3"), "-2.5e3");
    }

    #[test]
    fn should_handle_uppercase_e_notation_numbers() {
        assert_eq!(fix_json("2.5E"), "2.5");
        assert_eq!(fix_json("2.5E-"), "2.5");
        assert_eq!(fix_json("2.5E3"), "2.5E3");
        assert_eq!(fix_json("-2.5E3"), "-2.5E3");
    }

    #[test]
    fn should_handle_incomplete_numbers_with_exponent() {
        assert_eq!(fix_json("12.e"), "12");
        assert_eq!(fix_json("12.34e"), "12.34");
        assert_eq!(fix_json("5e"), "5");
    }
}

mod fix_json_string {
    use super::*;

    #[test]
    fn should_handle_incomplete_strings() {
        assert_eq!(fix_json("\"abc"), "\"abc\"");
    }

    #[test]
    fn should_handle_escape_sequences() {
        // TS: '"value with \\"quoted\\" text and \\\\ escape'
        let input = "\"value with \\\"quoted\\\" text and \\\\ escape\"";
        assert_eq!(fix_json(input), input);
    }

    #[test]
    fn should_handle_incomplete_escape_sequences() {
        // TS: '"value with \\'  -> '"value with "'
        assert_eq!(fix_json("\"value with \\"), "\"value with \"");
    }

    #[test]
    fn should_handle_incomplete_unicode_escape_sequences() {
        assert_eq!(fix_json("\"\\u"), "\"\"");
        assert_eq!(fix_json("\"\\u12"), "\"\"");
        assert_eq!(fix_json("\"text \\u00"), "\"text \"");
        assert_eq!(fix_json("{\"a\":\"\\u12"), "{\"a\":\"\"}");

        // assert.doesNotThrow(() => JSON.parse(json)) for each of the above.
        for json in [
            fix_json("\"\\u"),
            fix_json("\"\\u12"),
            fix_json("\"text \\u00"),
            fix_json("{\"a\":\"\\u12"),
        ] {
            assert!(
                serde_json::from_str::<serde_json::Value>(&json).is_ok(),
                "expected fixed JSON to parse: {json}"
            );
        }
    }

    #[test]
    fn should_handle_unicode_characters() {
        // TS: '"value with unicode \u003C"'  ('\u003C' is '<')
        let input = "\"value with unicode <\"";
        assert_eq!(fix_json(input), input);
    }
}

mod fix_json_array {
    use super::*;

    #[test]
    fn should_handle_incomplete_array() {
        assert_eq!(fix_json("["), "[]");
    }

    #[test]
    fn should_handle_closing_bracket_after_number_in_array() {
        assert_eq!(fix_json("[[1], [2"), "[[1], [2]]");
    }

    #[test]
    fn should_handle_closing_bracket_after_string_in_array() {
        assert_eq!(fix_json("[[\"1\"], [\"2"), "[[\"1\"], [\"2\"]]");
    }

    #[test]
    fn should_handle_closing_bracket_after_literal_in_array() {
        assert_eq!(fix_json("[[false], [nu"), "[[false], [null]]");
    }

    #[test]
    fn should_handle_closing_bracket_after_array_in_array() {
        assert_eq!(fix_json("[[[]], [[]"), "[[[]], [[]]]");
    }

    #[test]
    fn should_handle_closing_bracket_after_object_in_array() {
        assert_eq!(fix_json("[[{}], [{"), "[[{}], [{}]]");
    }

    #[test]
    fn should_handle_trailing_comma() {
        assert_eq!(fix_json("[1, "), "[1]");
    }

    #[test]
    fn should_handle_closing_array() {
        assert_eq!(fix_json("[[], 123"), "[[], 123]");
    }
}

mod fix_json_object {
    use super::*;

    #[test]
    fn should_handle_keys_without_values() {
        assert_eq!(fix_json("{\"key\":"), "{}");
    }

    #[test]
    fn should_handle_closing_brace_after_number_in_object() {
        assert_eq!(
            fix_json("{\"a\": {\"b\": 1}, \"c\": {\"d\": 2"),
            "{\"a\": {\"b\": 1}, \"c\": {\"d\": 2}}"
        );
    }

    #[test]
    fn should_handle_closing_brace_after_string_in_object() {
        assert_eq!(
            fix_json("{\"a\": {\"b\": \"1\"}, \"c\": {\"d\": 2"),
            "{\"a\": {\"b\": \"1\"}, \"c\": {\"d\": 2}}"
        );
    }

    #[test]
    fn should_handle_closing_brace_after_literal_in_object() {
        assert_eq!(
            fix_json("{\"a\": {\"b\": false}, \"c\": {\"d\": 2"),
            "{\"a\": {\"b\": false}, \"c\": {\"d\": 2}}"
        );
    }

    #[test]
    fn should_handle_closing_brace_after_array_in_object() {
        assert_eq!(
            fix_json("{\"a\": {\"b\": []}, \"c\": {\"d\": 2"),
            "{\"a\": {\"b\": []}, \"c\": {\"d\": 2}}"
        );
    }

    #[test]
    fn should_handle_closing_brace_after_object_in_object() {
        assert_eq!(
            fix_json("{\"a\": {\"b\": {}}, \"c\": {\"d\": 2"),
            "{\"a\": {\"b\": {}}, \"c\": {\"d\": 2}}"
        );
    }

    #[test]
    fn should_handle_partial_keys_first_key() {
        assert_eq!(fix_json("{\"ke"), "{}");
    }

    #[test]
    fn should_handle_partial_keys_second_key() {
        assert_eq!(fix_json("{\"k1\": 1, \"k2"), "{\"k1\": 1}");
    }

    #[test]
    fn should_handle_partial_keys_with_colon_second_key() {
        assert_eq!(fix_json("{\"k1\": 1, \"k2\":"), "{\"k1\": 1}");
    }

    #[test]
    fn should_handle_trailing_whitespace() {
        assert_eq!(fix_json("{\"key\": \"value\"  "), "{\"key\": \"value\"}");
    }

    #[test]
    fn should_handle_closing_after_empty_object() {
        assert_eq!(fix_json("{\"a\": {\"b\": {}"), "{\"a\": {\"b\": {}}}");
    }
}

mod fix_json_nesting {
    use super::*;

    #[test]
    fn should_handle_nested_arrays_with_numbers() {
        assert_eq!(fix_json("[1, [2, 3, ["), "[1, [2, 3, []]]");
    }

    #[test]
    fn should_handle_nested_arrays_with_literals() {
        assert_eq!(fix_json("[false, [true, ["), "[false, [true, []]]");
    }

    #[test]
    fn should_handle_nested_objects() {
        assert_eq!(fix_json("{\"key\": {\"subKey\":"), "{\"key\": {}}");
    }

    #[test]
    fn should_handle_nested_objects_with_numbers() {
        assert_eq!(
            fix_json("{\"key\": 123, \"key2\": {\"subKey\":"),
            "{\"key\": 123, \"key2\": {}}"
        );
    }

    #[test]
    fn should_handle_nested_objects_with_literals() {
        assert_eq!(
            fix_json("{\"key\": null, \"key2\": {\"subKey\":"),
            "{\"key\": null, \"key2\": {}}"
        );
    }

    #[test]
    fn should_handle_arrays_within_objects() {
        assert_eq!(fix_json("{\"key\": [1, 2, {"), "{\"key\": [1, 2, {}]}");
    }

    #[test]
    fn should_handle_objects_within_arrays() {
        assert_eq!(
            fix_json("[1, 2, {\"key\": \"value\","),
            "[1, 2, {\"key\": \"value\"}]"
        );
    }

    #[test]
    fn should_handle_nested_arrays_and_objects() {
        assert_eq!(
            fix_json("{\"a\": {\"b\": [\"c\", {\"d\": \"e\","),
            "{\"a\": {\"b\": [\"c\", {\"d\": \"e\"}]}}"
        );
    }

    #[test]
    fn should_handle_deeply_nested_objects() {
        assert_eq!(
            fix_json("{\"a\": {\"b\": {\"c\": {\"d\":"),
            "{\"a\": {\"b\": {\"c\": {}}}}"
        );
    }

    #[test]
    fn should_handle_potential_nested_arrays_or_objects() {
        assert_eq!(fix_json("{\"a\": 1, \"b\": ["), "{\"a\": 1, \"b\": []}");
        assert_eq!(fix_json("{\"a\": 1, \"b\": {"), "{\"a\": 1, \"b\": {}}");
        assert_eq!(fix_json("{\"a\": 1, \"b\": \""), "{\"a\": 1, \"b\": \"\"}");
    }
}

mod fix_json_regression {
    use super::*;

    #[test]
    fn should_handle_complex_nesting_1() {
        let input = concat!(
            "{\n",
            "  \"a\": [\n",
            "    {\n",
            "      \"a1\": \"v1\",\n",
            "      \"a2\": \"v2\",\n",
            "      \"a3\": \"v3\"\n",
            "    }\n",
            "  ],\n",
            "  \"b\": [\n",
            "    {\n",
            "      \"b1\": \"n",
        );
        let expected = concat!(
            "{\n",
            "  \"a\": [\n",
            "    {\n",
            "      \"a1\": \"v1\",\n",
            "      \"a2\": \"v2\",\n",
            "      \"a3\": \"v3\"\n",
            "    }\n",
            "  ],\n",
            "  \"b\": [\n",
            "    {\n",
            "      \"b1\": \"n\"}]}",
        );
        assert_eq!(fix_json(input), expected);
    }

    #[test]
    fn should_handle_empty_objects_inside_nested_objects_and_arrays() {
        assert_eq!(
            fix_json("{\"type\":\"div\",\"children\":[{\"type\":\"Card\",\"props\":{}"),
            "{\"type\":\"div\",\"children\":[{\"type\":\"Card\",\"props\":{}}]}"
        );
    }
}

// ===========================================================================
// parse-partial-json.test.ts
// ===========================================================================

mod parse_partial_json {
    use super::*;

    #[test]
    fn should_handle_nullish_input() {
        let result = parse_partial_json(None);
        assert_eq!(result.value, None);
        assert_eq!(result.state, ParsePartialJsonState::UndefinedInput);
    }

    #[test]
    fn should_parse_valid_json() {
        let valid_json = r#"{"key": "value"}"#;
        let result = parse_partial_json(Some(valid_json));
        assert_eq!(result.value, Some(json!({"key": "value"})));
        assert_eq!(result.state, ParsePartialJsonState::SuccessfulParse);
        // TS also asserts `safeParseJSON` was called with `{ text: validJson }`;
        // skipped: no mock in the Rust port (real serde_json parse is used).
    }

    #[test]
    fn should_repair_and_parse_partial_json() {
        let partial_json = r#"{"key": "value""#;
        let result = parse_partial_json(Some(partial_json));
        assert_eq!(result.value, Some(json!({"key": "value"})));
        assert_eq!(result.state, ParsePartialJsonState::RepairedParse);
        // TS also asserts fixJson was called with the partial input and the
        // second parse used the fixed text; skipped: no mock in the Rust port.
    }

    #[test]
    fn should_handle_invalid_json_that_cannot_be_repaired() {
        let invalid_json = "not json at all";
        let result = parse_partial_json(Some(invalid_json));
        assert_eq!(result.value, None);
        assert_eq!(result.state, ParsePartialJsonState::FailedParse);
        // TS mocks fixJson to return the input unchanged; the real fixJson
        // reduces this input to "n", which still fails to parse, so the
        // observable outcome (FailedParse) is identical.
    }
}

// ===========================================================================
// cosine-similarity.test.ts
// ===========================================================================

mod cosine_similarity_tests {
    use super::*;

    #[test]
    fn should_calculate_cosine_similarity_correctly() {
        let vector1 = vec![1.0, 2.0, 3.0];
        let vector2 = vec![4.0, 5.0, 6.0];
        let result = cosine_similarity(&vector1, &vector2).unwrap();
        // toBeCloseTo(0.9746318461970762, 5) -> |diff| < 5e-6.
        assert!((result - 0.9746318461970762).abs() < 5e-6, "got {result}");
    }

    #[test]
    fn should_calculate_negative_cosine_similarity_correctly() {
        let vector1 = vec![1.0, 0.0];
        let vector2 = vec![-1.0, 0.0];
        let result = cosine_similarity(&vector1, &vector2).unwrap();
        assert!((result - (-1.0)).abs() < 5e-6, "got {result}");
    }

    #[test]
    fn should_throw_an_error_when_vectors_have_different_lengths() {
        let vector1 = vec![1.0, 2.0, 3.0];
        let vector2 = vec![4.0, 5.0];
        // TS: expect(() => cosineSimilarity(...)).toThrowError();
        assert_eq!(
            cosine_similarity(&vector1, &vector2),
            Err(UtilError::VectorLengthMismatch)
        );
    }

    #[test]
    fn should_give_0_when_one_of_the_vectors_is_a_zero_vector() {
        let vector1 = vec![0.0, 1.0, 2.0];
        let vector2 = vec![0.0, 0.0, 0.0];

        let result = cosine_similarity(&vector1, &vector2).unwrap();
        assert_eq!(result, 0.0);

        let result2 = cosine_similarity(&vector2, &vector1).unwrap();
        assert_eq!(result2, 0.0);
    }

    #[test]
    fn should_handle_vectors_with_very_small_magnitudes() {
        let vector1 = vec![1e-10, 0.0, 0.0];
        let vector2 = vec![2e-10, 0.0, 0.0];
        let result = cosine_similarity(&vector1, &vector2).unwrap();
        // toBe(1) - exact. IEEE-754 f64 produces the same value as the JS test.
        assert_eq!(result, 1.0);

        let vector3 = vec![1e-10, 0.0, 0.0];
        let vector4 = vec![-1e-10, 0.0, 0.0];
        let result2 = cosine_similarity(&vector3, &vector4).unwrap();
        assert_eq!(result2, -1.0);
    }
}
