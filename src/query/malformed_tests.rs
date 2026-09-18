use super::*;

#[test]
fn rejects_malformed_json_query_corpus() {
    let cases = [
        (
            "truncated",
            include_str!("../../tests/corpus/json/truncated.json"),
            "invalid query JSON",
        ),
        (
            "unknown-field",
            include_str!("../../tests/corpus/json/unknown-field.json"),
            "unknown field",
        ),
        (
            "invalid-sort",
            include_str!("../../tests/corpus/json/invalid-sort.json"),
            "must be 1 or -1",
        ),
        (
            "invalid-operator",
            include_str!("../../tests/corpus/json/invalid-operator.json"),
            "unsupported operator",
        ),
    ];

    for (name, fixture, message) in cases {
        let error = parse(fixture.as_bytes()).unwrap_err();
        assert!(
            error.to_string().contains(message),
            "{name}: expected {message:?}, got {error}"
        );
    }
}
