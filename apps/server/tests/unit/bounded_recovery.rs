use super::*;

// Node prints an uncaught exception as a standalone `<Name>: <message>` line
// after the source excerpt and caret. This is the GH-90 R3 integration output.
const NODE_NOT_A_FUNCTION: &str = "[eval]:12\n assert.equal(local.wireVersion(),2,'AssertionError: mixed contract requires Node wire version 2');\n                    ^\n\nTypeError: local.wireVersion is not a function\n    at [eval]:12:21\n\nNode.js v24.18.0\n";

#[test]
fn not_a_function_line_is_a_code_diagnostic_only_in_canonical_form() {
    assert!(code_diagnostic_line(
        "TypeError: local.wireVersion is not a function"
    ));
    for line in [
        "TypeError: Cannot read properties of undefined (reading 'x')",
        "TypeError: local.wireVersion is not a function; retrying",
        "    at TypeError: local.wireVersion is not a function",
        "TypeError:local.wireVersion is not a function",
        "RangeError: local.wireVersion is not a function",
        "",
    ] {
        assert!(!code_diagnostic_line(line), "{line:?}");
    }
}

#[test]
fn existing_line_diagnostics_are_preserved() {
    assert!(code_diagnostic_line(
        "AssertionError [ERR_ASSERTION]: 1 == 2"
    ));
    assert!(code_diagnostic_line(
        "FAIL independent GH-88 acceptance contract: ready"
    ));
    assert!(code_diagnostic_line(
        "FAIL independent GH-88 integration contract: ready"
    ));
    assert!(!code_diagnostic_line(
        "FAIL independent GH-88 security contract: policy changed"
    ));
    assert!(!code_diagnostic_line(
        "FAIL dependent integration contract: x"
    ));
    assert!(!code_diagnostic_line("  AssertionError: indented"));
}

#[test]
fn markers_match_any_listed_substring() {
    assert!(contains_marker(
        "x connection reset y",
        &["refused", "reset"]
    ));
    assert!(!contains_marker("x connection y", &["refused", "reset"]));
    assert!(!contains_marker("anything", &[]));
}

#[test]
fn native_node_type_error_is_check_exit_after_infrastructure_markers() {
    assert_eq!(native_failure(NODE_NOT_A_FUNCTION), "check_exit");
    // The excerpt alone (quoted AssertionError text, caret) is not classified.
    let excerpt =
        NODE_NOT_A_FUNCTION.replace("TypeError: local.wireVersion is not a function\n", "");
    assert_eq!(native_failure(&excerpt), "unknown");
    assert_eq!(
        native_failure(&format!("permission denied\n{NODE_NOT_A_FUNCTION}")),
        "permission_denied"
    );
    assert_eq!(
        native_failure(&format!("connection refused\n{NODE_NOT_A_FUNCTION}")),
        "service_unavailable"
    );
    assert_eq!(
        native_failure(&format!("HTTP 429\n{NODE_NOT_A_FUNCTION}")),
        "rate_limited"
    );
    // Mentioning the text inside a source excerpt is not a diagnostic.
    assert_eq!(
        native_failure(" check('TypeError: x is not a function')\n"),
        "unknown"
    );
    assert_eq!(
        native_failure("TypeError: Cannot read properties of undefined\n"),
        "unknown"
    );
}
