use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiagnosticsComparison {
    pub baseline_count: usize,
    pub candidate_count: usize,
    pub intersection_count: usize,
    pub similarity_percent: f64,
}

pub fn compare_diagnostics(baseline: &[String], candidate: &[String]) -> DiagnosticsComparison {
    let left: BTreeSet<String> = baseline
        .iter()
        .map(|line| normalize_diagnostic_line(line.as_str()))
        .collect();
    let right: BTreeSet<String> = candidate
        .iter()
        .map(|line| normalize_diagnostic_line(line.as_str()))
        .collect();

    let intersection_count = left.intersection(&right).count();
    let union_count = left.union(&right).count();

    let similarity_percent = if union_count == 0 {
        100.0
    } else {
        (intersection_count as f64 / union_count as f64) * 100.0
    };

    DiagnosticsComparison {
        baseline_count: left.len(),
        candidate_count: right.len(),
        intersection_count,
        similarity_percent,
    }
}

pub fn extract_diagnostic_lines(stderr: &str) -> Vec<String> {
    let mut extracted = Vec::new();
    let mut current_block: Vec<String> = Vec::new();

    for raw_line in stderr.lines() {
        let line = raw_line.trim_end();
        if is_diagnostic_lead(line) {
            if !current_block.is_empty() {
                extracted.push(current_block.join("\n"));
                current_block.clear();
            }
            current_block.push(line.to_string());
            continue;
        }

        if !current_block.is_empty() && is_diagnostic_continuation(line) {
            current_block.push(line.to_string());
            continue;
        }

        if !current_block.is_empty() {
            extracted.push(current_block.join("\n"));
            current_block.clear();
        }
    }

    if !current_block.is_empty() {
        extracted.push(current_block.join("\n"));
    }

    extracted
}

fn normalize_diagnostic_line(line: &str) -> String {
    line.lines()
        .map(str::trim_end)
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn is_diagnostic_lead(line: &str) -> bool {
    const DIAGNOSTIC_PREFIXES: [&str; 6] =
        ["error:", "warn:", "warning:", "note:", "help:", "hint:"];
    let trimmed = line.trim_start();
    let lowered = trimmed.to_ascii_lowercase();
    if DIAGNOSTIC_PREFIXES
        .iter()
        .any(|prefix| lowered.starts_with(prefix))
    {
        return true;
    }
    if trimmed.starts_with("Plugin diagnostic") {
        return true;
    }
    trimmed.starts_with("{")
        && trimmed.contains("\"level\"")
        && (trimmed.contains("\"error\"")
            || trimmed.contains("\"warning\"")
            || trimmed.contains("\"warn\""))
}

fn is_diagnostic_continuation(line: &str) -> bool {
    if line.trim().is_empty() {
        return false;
    }
    line.starts_with(' ')
        || line.starts_with('\t')
        || line.trim_start().starts_with("-->")
        || line.trim_start().starts_with('|')
        || line.trim_start().starts_with('=')
}

#[cfg(test)]
mod tests {
    use super::{compare_diagnostics, extract_diagnostic_lines};

    #[test]
    fn similarity_is_full_for_equal_diagnostics() {
        let a = vec!["error: foo".to_string(), "warn: bar".to_string()];
        let b = vec!["warn: bar".to_string(), "error: foo".to_string()];

        let cmp = compare_diagnostics(&a, &b);
        assert_eq!(cmp.similarity_percent, 100.0);
        assert_eq!(cmp.intersection_count, 2);
    }

    #[test]
    fn extract_only_relevant_lines() {
        let stderr = "Compiling x\nwarn: one\nnoise\nerror: two";
        let lines = extract_diagnostic_lines(stderr);
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().any(|l| l == "warn: one"));
        assert!(lines.iter().any(|l| l == "error: two"));
    }

    #[test]
    fn extract_multiline_diagnostics_blocks() {
        let stderr = "error: failed\n --> src/lib.cairo:1:1\n  |\n  | bad\nok";
        let lines = extract_diagnostic_lines(stderr);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("error: failed"));
        assert!(lines[0].contains("--> src/lib.cairo:1:1"));
    }
}
