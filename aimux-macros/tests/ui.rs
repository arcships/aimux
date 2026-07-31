#[test]
fn tool_signature_contract() {
    let tests = trybuild::TestCases::new();
    tests.pass("tests/ui/pass.rs");
    tests.compile_fail("tests/ui/fail_wrong_args.rs");
    tests.compile_fail("tests/ui/fail_wrong_return.rs");
}
