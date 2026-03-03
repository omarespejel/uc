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
    let left: BTreeSet<String> = baseline.iter().map(normalize_diagnostic_line).collect();
    let right: BTreeSet<String> = candidate.iter().map(normalize_diagnostic_line).collect();

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
    const DIAGNOSTIC_PREFIXES: [&str; 6] =
        ["error:", "warn:", "warning:", "note:", "help:", "hint:"];

    stderr
        .lines()
        .map(str::trim)
        .filter(|line| {
            DIAGNOSTIC_PREFIXES
                .iter()
                .any(|prefix| line.starts_with(prefix))
                || line.starts_with("Plugin diagnostic")
        })
        .map(str::to_string)
        .collect()
}

fn normalize_diagnostic_line(line: &String) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
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
}
