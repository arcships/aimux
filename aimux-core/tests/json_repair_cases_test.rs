//! Regression cases for `json_repair`.
//!
//! `fixtures/json_repair_cases.jsonl` holds 740 inputs with the result the
//! line-for-line port of `fix-json.ts` / `parse-partial-json.ts` gave for
//! them: one shortest input per (outcome, last two characters, unchanged by
//! `fix_json`) bucket out of 300 000 generated ones, plus hand-picked edges
//! (truncated `\u` escapes, bare `-`, astral characters, partial literals,
//! array separators).
//! The upstream test cases themselves are in `util_test.rs`.

use aimux_core::json_repair::{PartialJson, fix_json};
use serde_json::Value;

#[test]
fn matches_the_recorded_cases() {
    let cases = include_str!("fixtures/json_repair_cases.jsonl");
    let mut checked = 0;
    let mut differences = Vec::new();
    for line in cases.lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let input = case["input"].as_str().unwrap();

        let fixed = fix_json(input);
        if fixed != case["fixed"] {
            differences.push(format!(
                "fix_json({input:?}) = {fixed:?}, want {}",
                case["fixed"]
            ));
        }

        let want = match case["state"].as_str().unwrap() {
            "parsed" => PartialJson::Parsed(case["value"].clone()),
            "repaired" => PartialJson::Repaired(case["value"].clone()),
            _ => PartialJson::Failed,
        };
        let got = PartialJson::parse(input);
        if got != want {
            differences.push(format!(
                "PartialJson::parse({input:?}) = {got:?}, want {want:?}"
            ));
        }
        checked += 1;
    }
    assert_eq!(checked, 740);
    assert!(
        differences.is_empty(),
        "{} differences:\n{}",
        differences.len(),
        differences[..differences.len().min(10)].join("\n")
    );
}
