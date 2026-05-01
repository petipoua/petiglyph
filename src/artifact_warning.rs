use std::path::Path;

pub(crate) const INCOMPATIBLE_ARTIFACT_PREFIX: &str = "incompatible artifact detected";

fn quoted_path(path: &Path) -> String {
    let escaped = path.display().to_string().replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn artifact_reason(error_text_lower: &str) -> Option<&'static str> {
    if error_text_lower.contains("unsupported glyph lock version")
        || error_text_lower.contains("unsupported unicode registry version")
        || (error_text_lower.contains("failed to parse")
            && (error_text_lower.contains("petiglyph.lock")
                || error_text_lower.contains(".unicode-registry.json")
                || error_text_lower.contains(".petiglyph-install-")))
        || error_text_lower.contains("glyph lock project_id mismatch")
        || error_text_lower.contains("duplicate source_file in glyph lock")
        || error_text_lower.contains("duplicate codepoint in glyph lock")
        || error_text_lower.contains("glyph lock contains")
    {
        return Some("project metadata is incompatible with the current guardrails");
    }

    if error_text_lower.contains("hash collision while installing immutable artifact") {
        return Some("installed font artifact does not match expected immutable content");
    }

    if error_text_lower.contains("blocked install for")
        || error_text_lower.contains("blocked uninstall for")
        || error_text_lower.contains("blocked uninstall outside")
    {
        return Some("install path is occupied by an incompatible filesystem artifact");
    }

    None
}

pub(crate) fn incompatible_artifact_warning(
    error_text: &str,
    manifest_path: Option<&Path>,
) -> Option<String> {
    let lowered = error_text.to_ascii_lowercase();
    let reason = artifact_reason(&lowered)?;
    let repair_cmd = if let Some(path) = manifest_path {
        format!("petiglyph doctor --repair --manifest {}", quoted_path(path))
    } else {
        "petiglyph doctor --repair".to_string()
    };
    Some(format!(
        "{INCOMPATIBLE_ARTIFACT_PREFIX}: {reason}. run `{repair_cmd}` then retry."
    ))
}
