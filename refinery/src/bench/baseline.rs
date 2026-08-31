//! Holding a run against a committed baseline.
//!
//! The comparison is deliberately narrow: two reports over the *same* corpus
//! at the *same* record shape, case by case, on the figures a regression shows
//! up in — records a second, peak resident memory, published size. Anything
//! else is refused rather than compared on a guess, because a number that
//! silently compares two different workloads is worse than no number.
//!
//! Hosts are allowed to differ — a baseline is often captured on the machine
//! that will re-run it, but not always — and the rendered comparison names
//! both, so a cross-host reading is visible rather than implied.

use serde::{Deserialize, Serialize};

use super::{BenchError, BenchReport, CaseResult};

/// How one case fared against its baseline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseDelta {
    /// The case label, as both reports record it.
    pub label: String,
    /// Records a second now, over records a second in the baseline.
    pub throughput_ratio: Option<f64>,
    /// Peak resident memory now, over the baseline's.
    pub peak_rss_ratio: Option<f64>,
    /// Published bytes now, over the baseline's.
    pub output_ratio: Option<f64>,
    /// What regressed, one line each; empty when the case held its ground.
    pub regressions: Vec<String>,
    /// Why this case has no ratios, when it has none.
    pub note: Option<String>,
}

/// One run held against a baseline, case by case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    /// The share a figure may move by before it is called a regression.
    pub tolerance: f64,
    /// The host the baseline was captured on — `linux/aarch64`.
    pub baseline_host: String,
    /// The host this run was measured on.
    pub current_host: String,
    /// Every case in either report, baseline order first.
    pub cases: Vec<CaseDelta>,
}

impl Comparison {
    /// Compares `current` against `baseline`, allowing a `tolerance` drift.
    ///
    /// A `tolerance` of `0.25` calls a case regressed when it reads under 75%
    /// of the baseline's records a second, or needs over 125% of its peak
    /// memory or published bytes.
    ///
    /// # Errors
    ///
    /// Returns [`BenchError::Config`] when `tolerance` is not in `[0, 1)`, or
    /// when the two reports measured different corpora or record shapes —
    /// figures from different workloads do not compare, and pretending they do
    /// would hide the regression this exists to find.
    pub fn of(
        baseline: &BenchReport,
        current: &BenchReport,
        tolerance: f64,
    ) -> Result<Self, BenchError> {
        if !(tolerance.is_finite() && (0.0..1.0).contains(&tolerance)) {
            return Err(BenchError::config(format!(
                "a regression tolerance must be a fraction in [0, 1), not {tolerance}"
            )));
        }
        if baseline.corpus != current.corpus {
            return Err(BenchError::config(format!(
                "the baseline measured {} shards × {} records ({} bytes), this run {} shards × {} records ({} bytes)",
                baseline.corpus.shards,
                baseline.corpus.records_per_shard,
                baseline.corpus.bytes,
                current.corpus.shards,
                current.corpus.records_per_shard,
                current.corpus.bytes,
            )));
        }
        if baseline.record_shape != current.record_shape {
            return Err(BenchError::config(format!(
                "the baseline measured {}-byte records, this run {}-byte records",
                baseline.record_shape.bytes_per_record, current.record_shape.bytes_per_record,
            )));
        }

        let mut cases: Vec<CaseDelta> = baseline
            .cases
            .iter()
            .map(|was| match current.case(&was.label) {
                Some(now) => compared(was, now, tolerance),
                None => CaseDelta {
                    label: was.label.clone(),
                    throughput_ratio: None,
                    peak_rss_ratio: None,
                    output_ratio: None,
                    regressions: vec![format!(
                        "{} is in the baseline but was not measured in this run",
                        was.label
                    )],
                    note: Some("not measured in this run".to_string()),
                },
            })
            .collect();

        cases.extend(
            current
                .cases
                .iter()
                .filter(|now| baseline.case(&now.label).is_none())
                .map(|now| CaseDelta {
                    label: now.label.clone(),
                    throughput_ratio: None,
                    peak_rss_ratio: None,
                    output_ratio: None,
                    regressions: Vec::new(),
                    note: Some("not in the baseline".to_string()),
                }),
        );

        Ok(Self {
            tolerance,
            baseline_host: host_of(baseline),
            current_host: host_of(current),
            cases,
        })
    }

    /// Every regression the comparison found, one line each.
    #[must_use]
    pub fn regressions(&self) -> Vec<String> {
        self.cases
            .iter()
            .flat_map(|case| case.regressions.iter().cloned())
            .collect()
    }

    /// Whether every case held its ground.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.cases.iter().all(|case| case.regressions.is_empty())
    }

    /// The comparison as a failure when anything regressed.
    ///
    /// # Errors
    ///
    /// Returns [`BenchError::Regression`] naming every case that regressed, so
    /// a benchmark gate exits non-zero rather than printing a table nobody
    /// reads.
    pub fn assert_clean(&self) -> Result<(), BenchError> {
        if self.is_clean() {
            return Ok(());
        }
        Err(BenchError::regression(self.regressions().join("; ")))
    }

    /// The comparison as Markdown, for a pull request or a job summary.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# Refinery benchmark — against the baseline\n\n");
        out.push_str(&format!(
            "- **baseline host** — `{}`\n- **this run** — `{}`\n- **tolerance** — {:.0}%\n\n",
            self.baseline_host,
            self.current_host,
            self.tolerance * 100.0,
        ));
        if self.baseline_host != self.current_host {
            out.push_str(
                "The baseline was captured on another host, so these ratios compare hardware as much as code.\n\n",
            );
        }

        out.push_str("| case | records/s vs baseline | peak RSS vs baseline | output vs baseline | verdict |\n");
        out.push_str("| --- | --- | --- | --- | --- |\n");
        for case in &self.cases {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                case.label,
                ratio(case.throughput_ratio),
                ratio(case.peak_rss_ratio),
                ratio(case.output_ratio),
                verdict(case),
            ));
        }
        out.push('\n');
        for regression in self.regressions() {
            out.push_str(&format!("- **REGRESSED** — {regression}\n"));
        }
        out
    }
}

/// One case measured in both reports.
fn compared(was: &CaseResult, now: &CaseResult, tolerance: f64) -> CaseDelta {
    let mut regressions = Vec::new();

    let throughput_ratio = divide(now.records_per_second(), was.records_per_second());
    if let Some(ratio) = throughput_ratio {
        if ratio < 1.0 - tolerance {
            regressions.push(format!(
                "{} reads {:.0} records/s, {:.2}× the baseline's {:.0}",
                now.label,
                now.records_per_second(),
                ratio,
                was.records_per_second(),
            ));
        }
    }

    let peak_rss_ratio = match (now.peak_rss_kib, was.peak_rss_kib) {
        (Some(now_kib), Some(was_kib)) => divide(now_kib as f64, was_kib as f64),
        _ => None,
    };
    if let Some(ratio) = peak_rss_ratio {
        if ratio > 1.0 + tolerance {
            regressions.push(format!(
                "{} peaks at {} KiB, {:.2}× the baseline's {} KiB",
                now.label,
                now.peak_rss_kib.unwrap_or_default(),
                ratio,
                was.peak_rss_kib.unwrap_or_default(),
            ));
        }
    }

    let output_ratio = divide(now.output_bytes as f64, was.output_bytes as f64);
    if let Some(ratio) = output_ratio {
        if ratio > 1.0 + tolerance {
            regressions.push(format!(
                "{} published {} bytes, {:.2}× the baseline's {}",
                now.label, now.output_bytes, ratio, was.output_bytes,
            ));
        }
    }

    CaseDelta {
        label: now.label.clone(),
        throughput_ratio,
        peak_rss_ratio,
        output_ratio,
        regressions,
        note: None,
    }
}

/// `now ÷ was`, or `None` when the baseline figure is not a divisor.
fn divide(now: f64, was: f64) -> Option<f64> {
    (was > 0.0 && now.is_finite()).then(|| now / was)
}

/// A report's host, as one comparable string.
fn host_of(report: &BenchReport) -> String {
    format!("{}/{}", report.host.os, report.host.arch)
}

/// A ratio for the table, or a plain dash when there was nothing to divide.
fn ratio(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_string(), |value| format!("{value:.2}×"))
}

/// The word a case is reported under.
fn verdict(case: &CaseDelta) -> String {
    if !case.regressions.is_empty() {
        return "REGRESSED".to_string();
    }
    case.note.clone().unwrap_or_else(|| "ok".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(label: &str, elapsed_ms: u64, peak_rss_kib: u64, output_bytes: u64) -> CaseResult {
        CaseResult {
            label: label.to_string(),
            transform: "sample --rate 0.05".to_string(),
            repeats: 3,
            elapsed_ms,
            peak_rss_kib: Some(peak_rss_kib),
            peak_rss_method: "proc-vmhwm".to_string(),
            input_bytes: 1024 * 1024,
            output_bytes,
            records_read: 100_000,
            records_written: 5_000,
        }
    }

    fn report(cases: Vec<CaseResult>) -> BenchReport {
        BenchReport {
            tool: crate::manifest::ToolIdentity::current(),
            host: crate::soak::HostFacts::detect(),
            record_shape: crate::corpus::RecordShape::new(2, 1)
                .expect("valid shape")
                .into(),
            corpus: super::super::CorpusFacts {
                shards: 2,
                records_per_shard: 50_000,
                bytes: 1024 * 1024,
            },
            repeats: 3,
            cases,
            reference: None,
        }
    }

    #[test]
    fn a_run_within_tolerance_is_clean() {
        let baseline = report(vec![case("sample", 100, 12_000, 1_000)]);
        let current = report(vec![case("sample", 110, 13_000, 1_050)]);

        let comparison = Comparison::of(&baseline, &current, 0.25).expect("comparable");

        assert!(comparison.is_clean(), "{:?}", comparison.regressions());
        comparison.assert_clean().expect("a clean run passes");
        assert!(comparison.to_markdown().contains("| sample |"));
    }

    #[test]
    fn a_slower_run_is_a_regression() {
        let baseline = report(vec![case("sample", 100, 12_000, 1_000)]);
        let current = report(vec![case("sample", 200, 12_000, 1_000)]);

        let comparison = Comparison::of(&baseline, &current, 0.25).expect("comparable");

        assert_eq!(comparison.regressions().len(), 1);
        assert!(comparison.regressions()[0].contains("records/s"));
        assert!(comparison.assert_clean().is_err());
        assert!(comparison.to_markdown().contains("REGRESSED"));
    }

    #[test]
    fn a_hungrier_or_fatter_run_is_a_regression_too() {
        let baseline = report(vec![case("sample", 100, 12_000, 1_000)]);
        let current = report(vec![case("sample", 100, 24_000, 4_000)]);

        let comparison = Comparison::of(&baseline, &current, 0.25).expect("comparable");

        let regressions = comparison.regressions();
        assert_eq!(regressions.len(), 2, "{regressions:?}");
        assert!(regressions.iter().any(|line| line.contains("peaks at")));
        assert!(regressions.iter().any(|line| line.contains("published")));
    }

    #[test]
    fn a_case_that_stopped_being_measured_is_a_regression() {
        let baseline = report(vec![
            case("sample", 100, 12_000, 1_000),
            case("quantise", 100, 12_000, 1_000),
        ]);
        let current = report(vec![case("sample", 100, 12_000, 1_000)]);

        let comparison = Comparison::of(&baseline, &current, 0.25).expect("comparable");

        assert_eq!(comparison.regressions().len(), 1);
        assert!(comparison.regressions()[0].contains("quantise"));
    }

    #[test]
    fn a_case_the_baseline_never_had_is_reported_but_not_failed() {
        let baseline = report(vec![case("sample", 100, 12_000, 1_000)]);
        let current = report(vec![
            case("sample", 100, 12_000, 1_000),
            case("fuzz", 100, 12_000, 1_000),
        ]);

        let comparison = Comparison::of(&baseline, &current, 0.25).expect("comparable");

        assert!(comparison.is_clean());
        assert_eq!(comparison.cases.len(), 2);
        assert_eq!(
            comparison.cases[1].note.as_deref(),
            Some("not in the baseline")
        );
    }

    #[test]
    fn refuses_to_compare_two_different_workloads() {
        let baseline = report(vec![case("sample", 100, 12_000, 1_000)]);
        let mut current = report(vec![case("sample", 100, 12_000, 1_000)]);
        current.corpus.bytes *= 4;

        let error = Comparison::of(&baseline, &current, 0.25)
            .expect_err("different corpora do not compare");

        assert!(matches!(error, BenchError::Config { .. }), "{error:?}");
    }

    #[test]
    fn refuses_a_tolerance_that_is_not_a_gate() {
        let baseline = report(vec![case("sample", 100, 12_000, 1_000)]);

        for tolerance in [-0.5, 1.0, 2.0, f64::NAN, f64::INFINITY] {
            let error = Comparison::of(&baseline, &baseline, tolerance)
                .expect_err("a tolerance outside [0, 1) is not a gate");
            assert!(matches!(error, BenchError::Config { .. }), "{error:?}");
        }
    }
}
