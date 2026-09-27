use super::*;
use crate::diagnostics::tests::binding;
use std::{fs, os::unix::fs::symlink};
fn root() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "diagnostic-capture-{}",
        crate::process::new_identity().unwrap()
    ));
    fs::create_dir(&path).unwrap();
    path
}
#[test]
fn capture_redacts_before_paging_and_keeps_both_digests() {
    let root = root();
    let text = "first failure\nAuthorization: Bearer fixture-placeholder\n中文\nlast failure\n";
    fs::write(root.join("report.json"), text).unwrap();
    let c = capture(&root, "report.json", &binding(), 1024, false, 100).unwrap();
    assert_eq!(c.artifact.availability, Availability::Available);
    assert_eq!(c.raw.as_ref().unwrap(), text.as_bytes());
    assert_eq!(c.artifact.raw_sha256, Some(crate::validation::sha256(text)));
    assert_ne!(c.artifact.raw_sha256, c.artifact.export_sha256);
    let export = String::from_utf8(c.export.unwrap()).unwrap();
    assert!(export.contains("first failure") && export.contains("last failure"));
    assert!(!export.contains("fixture-placeholder"));
    assert_eq!(c.artifact.original_bytes, Some(text.len() as u64));
    let partial = capture(&root, "report.json", &binding(), 30, false, 100).unwrap();
    assert_eq!(partial.artifact.availability, Availability::Partial);
    assert_eq!(partial.artifact.retained_bytes, 30);
    assert!(
        !String::from_utf8(partial.export.unwrap())
            .unwrap()
            .contains("Auth")
    );
    let interrupted = capture(&root, "report.json", &binding(), 1024, true, 100).unwrap();
    assert_eq!(interrupted.artifact.original_bytes, None);
    assert_eq!(interrupted.artifact.availability, Availability::Partial);
    fs::write(
        root.join("private"),
        format!("-----BEGIN {} KEY-----\nfixture-placeholder\n", "PRIVATE"),
    )
    .unwrap();
    let private = capture(&root, "private", &binding(), 1024, false, 100).unwrap();
    assert_eq!(private.export, Some(b"[redacted]".to_vec()));
}
#[test]
fn sources_cannot_escape_or_expand_compressed_payloads() {
    let root = root();
    fs::write(root.join("log"), "text").unwrap();
    symlink(root.join("log"), root.join("link")).unwrap();
    fs::create_dir(root.join("dir")).unwrap();
    fs::write(root.join("archive"), [0x1f, 0x8b, 0, 255]).unwrap();
    fs::write(root.join("zip"), b"PK\x03\x04aaaa").unwrap();
    fs::write(root.join("binary"), [0, 1, 2]).unwrap();
    for path in [
        "missing",
        "link",
        "dir",
        "archive",
        "zip",
        "binary",
        "../log",
        "/etc/passwd",
        "https://example.com",
        "x\\y",
    ] {
        let c = capture(&root, path, &binding(), 1024, false, 100).unwrap();
        assert_eq!(c.artifact.availability, Availability::Missing, "{path}");
        assert!(c.export.is_none());
    }
    assert!(read(&root, "log", crate::diagnostics::MAX_FILE + 1).is_err());
    assert!(stamp(&fs::metadata(root.join("log")).unwrap()).2 > 0);
}
#[test]
fn partial_single_line_never_discloses_an_unfinished_marker() {
    let root = root();
    fs::write(
        root.join("json"),
        "{\"failure\":\"first\",\"password\":\"fixture-placeholder\"}",
    )
    .unwrap();
    let c = capture(&root, "json", &binding(), 20, false, 100).unwrap();
    assert_eq!(c.export, Some(Vec::new()));
    fs::write(root.join("utf8"), [b'a', 0xff, b'b']).unwrap();
    let c = capture(&root, "utf8", &binding(), 10, false, 100).unwrap();
    assert_eq!(String::from_utf8(c.export.unwrap()).unwrap(), "a�b");
}
#[test]
fn descriptor_capture_detects_replacement_and_mutation() {
    let root = root();
    fs::write(root.join("log"), "original\n").unwrap();
    let directory = Directory::open(&root).unwrap();
    let file = directory.read(Path::new("log")).unwrap();
    let before = file.metadata().unwrap();
    fs::write(root.join("replacement"), "replacement\n").unwrap();
    fs::rename(root.join("replacement"), root.join("log")).unwrap();
    assert!(unchanged(&directory, "log", &before, &file).is_err());
    let file = directory.read(Path::new("log")).unwrap();
    let before = file.metadata().unwrap();
    fs::write(root.join("log"), "changed length again\n").unwrap();
    assert!(unchanged(&directory, "log", &before, &file).is_err());
}
#[test]
fn long_secret_line_is_redacted_before_small_byte_ranges() {
    let root = root();
    let text = format!(
        "FIRST\n{}password=fixture-placeholder\nLAST\n",
        "x".repeat(10000)
    );
    fs::write(root.join("report.md"), &text).unwrap();
    let captured = capture(&root, "report.md", &binding(), 32768, false, 100).unwrap();
    let bytes = captured.export.unwrap();
    let mut rebuilt = Vec::new();
    let mut offset = 0;
    loop {
        let chunk =
            crate::diagnostics::chunk(captured.artifact.clone(), &bytes, offset, 4).unwrap();
        rebuilt.extend_from_slice(chunk.text.as_bytes());
        offset = chunk.next;
        if chunk.end {
            break;
        }
    }
    let output = String::from_utf8(rebuilt).unwrap();
    assert!(output.contains("FIRST") && output.contains("LAST"));
    assert!(!output.contains("fixture-placeholder"));
}

#[test]
fn untrusted_filenames_are_redacted_in_manifests() {
    let root = root();
    fs::write(root.join("token=fixture-placeholder"), "failure").unwrap();
    let capture = capture(
        &root,
        "token=fixture-placeholder",
        &binding(),
        100,
        false,
        100,
    )
    .unwrap();
    assert_eq!(capture.artifact.purpose, "[redacted]");
    assert_eq!(capture.artifact.availability, Availability::Available);
    assert!(!crate::extension_contract::valid_relative_path("x\n"));
    assert!(!crate::extension_contract::valid_relative_path(
        &"x".repeat(4097)
    ));
}

#[test]
fn structured_credentials_are_redacted_across_lines_and_nested_values() {
    let root = root();
    let text = "{\n\"failures\":[\"FIRST\",\"LAST\"],\n\"password\":\n\"fixture-placeholder\",\n\"nested\":[{\"token\":[\"fixture-placeholder\"],\"count\":1,\"ok\":true,\"none\":null}]\n}";
    fs::write(root.join("report.json"), text).unwrap();
    let captured = capture(&root, "report.json", &binding(), 1024, false, 100).unwrap();
    let bytes = captured.export.unwrap();
    let exported: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(exported["password"], "[redacted]");
    assert_eq!(exported["nested"][0]["token"], "[redacted]");
    assert_eq!(exported["failures"], serde_json::json!(["FIRST", "LAST"]));
    assert_eq!(exported["nested"][0]["count"], 1);
    assert!(
        !String::from_utf8(bytes)
            .unwrap()
            .contains("fixture-placeholder")
    );
    assert_ne!(
        captured.artifact.raw_sha256,
        captured.artifact.export_sha256
    );
    let cutoff = text.find(",\n\"nested\"").unwrap() + 2;
    let partial = capture(&root, "report.json", &binding(), cutoff as u64, false, 100).unwrap();
    assert_eq!(partial.artifact.availability, Availability::Partial);
    assert_eq!(partial.export, Some(b"[redacted]".to_vec()));
    assert!(structured_redaction("not-json").is_none());
    assert_eq!(
        redacted(b"{\"healthy\":true,\"count\":1,\"nothing\":null}", false),
        b"{\"healthy\":true,\"count\":1,\"nothing\":null}"
    );
}
